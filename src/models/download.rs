use super::{
    File, Model, ModelError,
    hub::{Hub, network_error, validate_path},
};
use crate::{paths::Paths, store::Store};
use anyhow::{Result, ensure};
use reqwest::{
    StatusCode,
    header::{CONTENT_RANGE, RANGE},
};
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn issue(code: &'static str, message: &'static str, next_step: &'static str) -> ModelError {
    ModelError {
        code,
        message,
        retryable: false,
        next_step,
        details: json!({}),
    }
}

pub fn verify(path: &Path, file: &File) -> Result<bool> {
    verify_with(path, file, || Ok(()))
}
pub(crate) fn verify_with(
    path: &Path,
    file: &File,
    mut tick: impl FnMut() -> Result<()>,
) -> Result<bool> {
    reject_symlinks(path)?;
    if !path.exists() {
        return Ok(false);
    }
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "model file cannot be a symlink"
    );
    if path.metadata()?.len() != file.size {
        return Ok(false);
    }
    let use_sha256 = match file.algorithm.as_str() {
        "sha256" => true,
        "git_sha1" => false,
        _ => anyhow::bail!("unsupported hash algorithm"),
    };
    let mut input = fs::File::open(path)?;
    let mut sha256 = Sha256::new();
    let mut sha1 = Sha1::new();
    sha1.update(format!("blob {}\0", file.size).as_bytes());
    let mut buffer = [0u8; 65536];
    loop {
        tick()?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        if use_sha256 {
            sha256.update(&buffer[..n]);
        } else {
            sha1.update(&buffer[..n]);
        }
    }
    let actual = match file.algorithm.as_str() {
        "sha256" => format!("{:x}", sha256.finalize()),
        "git_sha1" => format!("{:x}", sha1.finalize()),
        _ => anyhow::bail!("unsupported hash algorithm"),
    };
    Ok(actual == file.hash)
}

fn reject_symlinks(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(ancestor) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "managed download path contains a symlink"
            );
        }
    }
    Ok(())
}

/// Download one pinned file, preserving partial bytes after interruption.
pub fn fetch_file(hub: &Hub, model: &Model, file: &File, root: &Path) -> Result<()> {
    validate_path(&file.path)?;
    let target = root.join(&file.path);
    reject_symlinks(&target)?;
    if verify(&target, file)? {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = target.with_file_name(format!(
        "{}.part",
        target.file_name().unwrap().to_string_lossy()
    ));
    reject_symlinks(&partial)?;
    let mut offset = partial.metadata().map(|m| m.len()).unwrap_or(0);
    if offset > file.size {
        fs::remove_file(&partial)?;
        offset = 0;
    }
    if offset == file.size && verify(&partial, file)? {
        fs::rename(partial, target)?;
        return Ok(());
    }
    if offset == file.size {
        fs::remove_file(&partial)?;
        offset = 0;
    }
    let url = hub.download_url(model, file)?;
    let mut request = hub.request(url).header("Accept-Encoding", "identity");
    if offset > 0 {
        request = request.header(RANGE, format!("bytes={offset}-"));
    }
    let response = request.send().map_err(|_| network_error(None))?;
    if response.status() != StatusCode::OK && response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(network_error(Some(response.status().as_u16())).into());
    }
    if response.status() == StatusCode::PARTIAL_CONTENT {
        let expected = format!("bytes {offset}-{}/{}", file.size - 1, file.size);
        if response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            != Some(expected.as_str())
        {
            return Err(issue(
                "invalid_download_range",
                "The server returned an unexpected byte range.",
                "Retry using --endpoint hf; the partial download is retained.",
            )
            .into());
        }
    } else {
        offset = 0;
    }
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(offset == 0)
        .append(offset > 0)
        .open(&partial)?;
    let copied = std::io::copy(&mut response.take(file.size - offset + 1), &mut output)
        .map_err(|_| network_error(None))?;
    output.flush()?;
    output.sync_all()?;
    drop(output);
    if copied < file.size - offset {
        return Err(network_error(None).into());
    }
    if copied != file.size - offset || !verify(&partial, file)? {
        if partial.metadata()?.len() >= file.size {
            fs::remove_file(&partial)?;
        }
        return Err(issue("integrity_check_failed","Downloaded bytes do not match the pinned file manifest.","Retry the download. Incomplete files are retained; corrupt complete files are discarded.").into());
    }
    fs::rename(partial, target)?;
    Ok(())
}

fn check_space(remaining: u64, available: u64) -> Result<()> {
    let required = remaining.saturating_add(1024 * 1024);
    if available < required {
        return Err(ModelError {code:"insufficient_disk_space",message:"There is not enough free space for the model.",retryable:false,next_step:"Free disk space or set --models-dir to an absolute directory on another disk.",details:json!({"required_bytes":required,"available_bytes":available})}.into());
    }
    Ok(())
}

pub fn install(
    model: &Model,
    hub: &Hub,
    paths: &Paths,
    models_dir: Option<PathBuf>,
) -> Result<Value> {
    super::manifest::validate(model)?;
    if model.status == "retired" {
        return Err(ModelError {code:"model_retired",message:"This model is retired and cannot be downloaded.",retryable:false,next_step:"Use model info to find the suggested replacement, or search for an active model in the same family.",details:json!({"model":model.id,"replaced_by":model.replaced_by})}.into());
    }
    if model.files.is_empty() {
        return Err(issue("model_not_downloadable","This model has no downloadable file manifest.","Use the registered local files, or update the user model definition with download URLs, sizes and hashes.").into());
    }
    if let Some(existing) = Store::read_models(&paths.database())?
        .into_iter()
        .find(|m| m.name == model.id && m.ownership == "reference")
    {
        return Err(ModelError {code:"resource_conflict",message:"An external model reference already uses this catalog ID.",retryable:false,next_step:"Choose a different name for the external reference before downloading this catalog model.",details:json!({"name":existing.name})}.into());
    }
    let root = models_dir.unwrap_or_else(|| paths.data.join("models"));
    ensure!(root.is_absolute(), "models directory must be absolute");
    fs::create_dir_all(&root)?;
    let root = root.canonicalize()?.join(&model.id);
    reject_symlinks(&root)?;
    fs::create_dir_all(&root)?;
    let lock_path = root.join("download.lock");
    reject_symlinks(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| ModelError {
        code: "download_busy",
        message: "Another download is already using this model directory.",
        retryable: true,
        next_step: "Wait for the current download to finish, then retry.",
        details: json!({"model":model.id}),
    })?;
    let target = root.join(&model.revision);
    let stage = root.join(format!(".{}.partial", model.revision));
    reject_symlinks(&target)?;
    reject_symlinks(&stage)?;
    if target.exists() {
        for file in &model.files {
            if !verify(&target.join(&file.path), file)? {
                return Err(issue("integrity_check_failed","An installed model file is missing or corrupt.","Inspect the managed model directory; move the damaged revision aside and retry. External models are never changed.").into());
            }
        }
    } else {
        fs::create_dir_all(&stage)?;
        let required = model
            .files
            .iter()
            .try_fold(0u64, |total, file| -> Result<u64> {
                let target = stage.join(&file.path);
                if verify(&target, file)? {
                    return Ok(total);
                }
                let partial = target.with_file_name(format!(
                    "{}.part",
                    target.file_name().unwrap().to_string_lossy()
                ));
                reject_symlinks(&partial)?;
                let reusable = partial.metadata().map(|m| m.len()).unwrap_or(0);
                let remaining = if reusable < file.size {
                    file.size - reusable
                } else if reusable == file.size && verify(&partial, file)? {
                    0
                } else {
                    file.size
                };
                Ok(total + remaining)
            })?;
        // Count remaining bytes on the destination filesystem, not the cache disk.
        let free = fs2::available_space(&stage)?;
        check_space(required, free)?;
        for file in &model.files {
            fetch_file(hub, model, file, &stage)?;
        }
        let manifest = stage.join("gendock-manifest.json");
        reject_symlinks(&manifest)?;
        let mut output = fs::File::create(&manifest)?;
        output.write_all(&serde_json::to_vec_pretty(model)?)?;
        output.sync_all()?;
        fs::rename(&stage, &target)?;
    }
    let target = target.canonicalize()?;
    Store::open(&paths.database())?.register_managed(&model.id, &target, &model.backend)?;
    let ready = crate::image::supports(model)
        && crate::runtime::ImageRuntime::load(paths).is_ok()
        && crate::models::Device::detect().compatibility(model)["status"] == "candidate";
    Ok(
        json!({"id":model.id,"repository":model.repository,"revision":model.revision,"path":target,
        "ownership":"managed","download_status":"complete","integrity_checked":true,"inference_ready":ready,
        "endpoint":hub.endpoint.as_str()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verification_heartbeats_and_cancellation_preserve_model_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap().join("weights");
        let bytes = vec![42u8; 192 * 1024];
        std::fs::write(&path, &bytes).unwrap();
        let file:File=serde_json::from_value(json!({"path":"weights","size":bytes.len(),"algorithm":"sha256","hash":format!("{:x}",Sha256::digest(&bytes)),"url":"https://example.com/weights"})).unwrap();
        let mut ticks = 0;
        assert!(
            verify_with(&path, &file, || {
                ticks += 1;
                if ticks == 2 {
                    anyhow::bail!("cancelled")
                };
                Ok(())
            })
            .is_err()
        );
        assert_eq!(ticks, 2);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(verify(&path, &file).unwrap());
    }
    #[test]
    fn insufficient_space_is_actionable() {
        let error = check_space(1000, 999).unwrap_err();
        let detail = error.downcast_ref::<ModelError>().unwrap();
        assert_eq!(detail.code, "insufficient_disk_space");
        assert_eq!(detail.details["available_bytes"], 999);
        assert!(check_space(1000, 1024 * 1024 + 1000).is_ok());
    }
}
