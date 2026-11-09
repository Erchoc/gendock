//! Synchronous image service. CLI parsing and worker output stay outside this layer.
mod input;

use crate::{
    models::ModelError,
    paths::Paths,
    runtime::{ImageRuntime, mflux},
    store::Store,
};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    time::Instant,
};

pub const DEFAULT_MODEL: &str = "flux2-klein-4b-q4";
pub fn supports(model: &crate::models::Model) -> bool {
    model.id == DEFAULT_MODEL && model.backend == "mflux" && model.model_type == "image"
}
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub prompt: String,
    pub model: Option<String>,
    pub out: Option<PathBuf>,
    pub seed: Option<u64>,
    pub aspect: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub steps: Option<u32>,
}
fn problem(
    code: &'static str,
    message: &'static str,
    next: &'static str,
    details: Value,
) -> anyhow::Error {
    ModelError {
        code,
        message,
        retryable: false,
        next_step: next,
        details,
    }
    .into()
}

fn dimensions(options: &Options) -> Result<(u32, u32)> {
    let size = match (options.width, options.height, options.aspect.as_deref()) {
        (Some(w), Some(h), None) => (w, h),
        (None, None, None | Some("1:1")) => (512, 512),
        (None, None, Some("3:2")) => (768, 512),
        (None, None, Some("2:3")) => (512, 768),
        (None, None, Some("16:9")) => (1024, 576),
        (None, None, Some("9:16")) => (576, 1024),
        (None, None, Some("4:3")) => (768, 576),
        (None, None, Some("3:4")) => (576, 768),
        _ => {
            return Err(problem(
                "invalid_image_options",
                "Image dimensions or aspect ratio are invalid.",
                "Use --width and --height together, or a supported --aspect value, not both.",
                json!({"allowed_aspects":["1:1","3:2","2:3","16:9","9:16","4:3","3:4"]}),
            ));
        }
    };
    if [size.0, size.1]
        .iter()
        .any(|v| !(256..=2048).contains(v) || v % 16 != 0)
    {
        return Err(problem(
            "invalid_image_options",
            "Image dimensions must be multiples of 16 between 256 and 2048.",
            "Choose dimensions within the supported range.",
            json!({"width":size.0,"height":size.1}),
        ));
    }
    Ok(size)
}

pub struct Queued {
    pub id: i64,
    pub server_id: String,
    pub revision: String,
    pub shutdown: Arc<AtomicBool>,
}

pub fn recover_interrupted(store: &Store) -> Result<()> {
    recover_except(store, None)
}
fn recover_except(store: &Store, current: Option<i64>) -> Result<()> {
    // Caller holds the OS generation lock. Do not kill a surviving orphan or a reused PID.
    for (id, request) in store.running_sync_jobs()? {
        if current == Some(id) {
            continue;
        }
        if let Some(pid) = request["worker_pid"]
            .as_u64()
            .filter(|pid| *pid <= u32::MAX as u64)
        {
            let alive = std::process::Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?
                .success();
            if alive {
                return Err(problem(
                    "generation_busy",
                    "A previous image worker may still be running.",
                    "Inspect the previous job and its worker before submitting again. Do not blindly retry or kill unrelated processes.",
                    json!({"job_id":id,"worker_pid":pid}),
                ));
            }
        }
        if request["execution_mode"] == "queued"
            && request["queue"]["lease_until"].as_u64().unwrap_or(0)
                > std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs()
            && let Some(pid) = request["owner_pid"]
                .as_u64()
                .filter(|pid| *pid <= u32::MAX as u64)
            && std::process::Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?
                .success()
        {
            return Err(problem(
                "generation_busy",
                "A live service owns the next execution lease.",
                "Wait for the service to execute or release its claimed task.",
                json!({"job_id":id}),
            ));
        }
        store.fail_image_job(id,"interrupted",&json!({"code":"generation_interrupted","message":"The owning process exited before recording a result."}))?;
    }
    Ok(())
}

pub fn validate_input(path: &Path) -> Result<Value> {
    let input = input::Input::read(path)?;
    Ok(
        json!({"sha256":input.sha256,"width":input.width,"height":input.height,"format":input.extension}),
    )
}
pub fn validate_submission(
    options: &mut Options,
    input: Option<&Path>,
    kind: &str,
    paths: &Paths,
) -> Result<String> {
    dimensions(options)?;
    if options.prompt.trim().is_empty()
        || options.prompt.len() > 32768
        || !(1..=50).contains(&options.steps.unwrap_or(4))
        || options.seed.is_some_and(|seed| seed > u32::MAX as u64)
    {
        return Err(problem(
            "invalid_image_options",
            "Prompt, step count or seed is invalid.",
            "Use a nonempty prompt of at most 32768 bytes, 1–50 steps and a 32-bit seed.",
            json!({}),
        ));
    }
    if let Some(path) = input {
        validate_input(path)?;
    }
    let cfg = crate::config::Config::load_or_init(paths)?;
    let selected = options.model.as_deref().unwrap_or(if kind == "image.edit" {
        &cfg.defaults.image_edit
    } else {
        &cfg.defaults.image_generate
    });
    let store = Store::open(&paths.database())?;
    let model = store.catalog_find(selected)?;
    if !supports(&model) || !model.tasks.iter().any(|task| task == kind) {
        return Err(problem(
            "model_execution_unsupported",
            "This model does not have an integrated recipe for the task.",
            "Choose a supported model.",
            json!({"model":model.id}),
        ));
    }
    if model.status == "retired" || (options.model.is_none() && model.status != "active") {
        return Err(problem(
            "model_not_available",
            "The selected model is not available.",
            "Choose an active model.",
            json!({"model":model.id}),
        ));
    }
    let installed = store
        .models()?
        .into_iter()
        .find(|m| m.name == model.id)
        .ok_or_else(|| {
            problem(
                "model_not_installed",
                "The selected model is not registered locally.",
                "Download or register the model explicitly.",
                json!({"model":model.id}),
            )
        })?;
    if model.files.is_empty()
        || !model.files.iter().all(|file| {
            Path::new(&installed.path)
                .join(&file.path)
                .metadata()
                .is_ok_and(|m| m.is_file() && m.len() == file.size)
        })
    {
        return Err(problem(
            "model_files_incomplete",
            "The local model files are incomplete.",
            "Restore or register a complete model snapshot.",
            json!({"model":model.id}),
        ));
    }
    ImageRuntime::load_for(paths, kind == "image.edit")?;
    options.model = Some(model.id);
    Ok(model.revision)
}

pub fn generate(options: Options) -> Result<Value> {
    execute(options, None, None)
}
pub fn edit(options: Options, input: PathBuf) -> Result<Value> {
    execute(options, Some(input), None)
}

pub fn execute_queued(options: Options, input: Option<PathBuf>, queued: Queued) -> Result<Value> {
    execute(options, input, Some(queued))
}
fn execute(options: Options, input: Option<PathBuf>, queued: Option<Queued>) -> Result<Value> {
    let editing = input.is_some();
    let kind = if editing {
        "image.edit"
    } else {
        "image.generate"
    };
    let input = input.as_deref().map(input::Input::read).transpose()?;
    let (width, height) = dimensions(&options)?;
    let steps = options.steps.unwrap_or(4);
    if options.prompt.trim().is_empty()
        || !(1..=50).contains(&steps)
        || options.seed.is_some_and(|seed| seed > u32::MAX as u64)
    {
        return Err(problem(
            "invalid_image_options",
            "Prompt, step count or seed is invalid.",
            "Provide a nonempty prompt, 1–50 steps and a seed between 0 and 4294967295.",
            json!({}),
        ));
    }
    let paths = Paths::discover()?;
    let config = crate::config::Config::load_or_init(&paths)?;
    let default_export = if options.out.is_none() && queued.is_none() {
        Some((
            config
                .output
                .image_directory(editing, &std::env::current_dir()?),
            chrono::Local::now().format("image_%m%d_%H%M").to_string(),
        ))
    } else {
        None
    };
    let export = options
        .out
        .as_ref()
        .map(|out| -> Result<PathBuf> {
            let path = if out.is_absolute() {
                out.clone()
            } else {
                std::env::current_dir()?.join(out)
            };
            if path.extension().and_then(|v| v.to_str()) != Some("png") {
                return Err(problem(
                    "unsupported_output_format",
                    "Only PNG export is supported.",
                    "Use an output filename ending in .png.",
                    json!({}),
                ));
            }
            if fs::symlink_metadata(&path).is_ok() {
                return Err(problem(
                    "output_exists",
                    "The output path already exists.",
                    "Choose a new output path. Existing files are never overwritten.",
                    json!({"path":path}),
                ));
            }
            Ok(path)
        })
        .transpose()?;
    let configured = if editing {
        &config.defaults.image_edit
    } else {
        &config.defaults.image_generate
    };
    let store = Store::open(&paths.database())?;
    let model = store.catalog_find(options.model.as_deref().unwrap_or(configured))?;
    if let Some(queue) = &queued
        && model.revision != queue.revision
    {
        return Err(problem(
            "model_revision_changed",
            "The model definition changed after submission.",
            "Inspect the model version and submit a new task explicitly.",
            json!({"model":model.id}),
        ));
    }
    if !supports(&model) || !model.tasks.iter().any(|task| task == kind) {
        return Err(problem(
            "model_execution_unsupported",
            "This model does not have an integrated image recipe yet.",
            "Use flux2-klein-4b-q4 for this first image adapter.",
            json!({"model":model.id}),
        ));
    }
    if model.status != "active" && options.model.is_none() || model.status == "retired" {
        return Err(problem(
            "model_not_available",
            "The selected model is not available for this request.",
            "Inspect model status and select an active model explicitly.",
            json!({"model":model.id,"status":model.status}),
        ));
    }
    let installed=store.models()?.into_iter().find(|m|m.name==model.id).ok_or_else(||problem(
        "model_not_installed","The selected model has not been downloaded or registered locally.",
        "Run the download command in details, or register existing local files. Generation never downloads automatically.",
        json!({"model":model.id,"operation":kind,"config_path":paths.config.join("config.toml"),"download_command":["gendock","model","download","--model",model.id]})
    ))?;
    let runtime = ImageRuntime::load_for(&paths, editing)?;
    fs::create_dir_all(&paths.state)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(paths.state.join("image-generation.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| {
        problem(
            "generation_busy",
            "Another image generation is running.",
            "Wait for the current generation to finish before submitting again.",
            json!({}),
        )
    })?;
    recover_except(&store, queued.as_ref().map(|q| q.id))?;
    let model_path = PathBuf::from(&installed.path).canonicalize().map_err(|_|problem("model_files_incomplete","The registered model directory is unavailable.","Restore the local model files or correct the registration; no automatic download was started.",json!({"model":model.id,"path":installed.path})))?;
    crate::models::manifest::validate(&model)?;
    if model.files.is_empty() {
        return Err(problem(
            "model_files_incomplete",
            "The model definition has no file manifest.",
            "Use a supported complete model definition.",
            json!({"model":model.id}),
        ));
    }
    let mut preflight_tick = Instant::now();
    for file in &model.files {
        if !crate::models::download::verify_with(&model_path.join(&file.path), file, || {
            if let Some(queue) = &queued
                && Instant::now() >= preflight_tick
            {
                if queue.shutdown.load(Ordering::Relaxed) {
                    return Err(problem(
                        "generation_interrupted",
                        "The service stopped before execution.",
                        "Restart the service; submit a new task explicitly if this task was interrupted.",
                        json!({"job_id":queue.id}),
                    ));
                }
                if store.service_tick(queue.id, &queue.server_id)? {
                    return Err(problem(
                        "generation_cancelled",
                        "The queued task was cancelled before inference.",
                        "Submit a new task when ready.",
                        json!({"job_id":queue.id}),
                    ));
                }
                preflight_tick = Instant::now() + std::time::Duration::from_millis(500);
            }
            Ok(())
        })? {
            return Err(problem(
                "model_files_incomplete",
                "A required model file is missing or corrupt.",
                "Repair the model download or update the local reference to a complete snapshot.",
                json!({"model":model.id,"file":file.path}),
            ));
        }
    }
    let seed = options.seed.unwrap_or_else(|| {
        u32::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..4].try_into().unwrap()) as u64
    });
    let asset_id = uuid::Uuid::new_v4().to_string();
    let asset_dir = paths
        .data
        .join("assets")
        .join(&asset_id[..2])
        .join(&asset_id);
    let output = asset_dir.join("image.png");
    let log_dir = paths.state.join("logs/image");
    let log = log_dir.join(format!("{asset_id}.log"));
    let snapshot = input
        .as_ref()
        .map(|input| asset_dir.join(format!("input.{}", input.extension)));
    let input_metadata = input
        .as_ref()
        .zip(snapshot.as_ref())
        .map(|(input, snapshot)| input.metadata(snapshot));
    let request = json!({"execution_mode":"synchronous","operation":kind,"input":input_metadata,"model_selection":if options.model.is_some(){"argument"}else{"config"},"owner_pid":std::process::id(),"model":model.id,"model_revision":model.revision,
        "prompt":options.prompt,"seed":seed,"width":width,"height":height,"steps":steps,"runtime":{"adapter":runtime.adapter,"version":runtime.version,"executable_sha256":if editing{runtime.edit_executable_sha256.as_ref()}else{Some(&runtime.executable_sha256)}},"asset_id":asset_id,"output":output,"log_path":log,"export_requested":export,"default_export":default_export.as_ref().map(|(directory,stem)|json!({"directory":directory,"filename_stem":stem}))});
    let _sleep_guard = crate::power::SleepGuard::acquire(config.power.prevent_sleep)?;
    let signals = mflux::Signals::install()?;
    let job_id = if let Some(queue) = &queued {
        store.enrich_service_job(queue.id, &queue.server_id, &request)?;
        queue.id
    } else {
        store.start_image_job_for(kind, &request)?
    };
    let started = Instant::now();
    let operation = (|| -> Result<Value> {
        fs::create_dir_all(&asset_dir)?;
        fs::create_dir_all(&log_dir)?;
        if let Some((input, snapshot)) = input.as_ref().zip(snapshot.as_ref()) {
            fs::write(snapshot, &input.bytes)?;
            fs::File::open(snapshot)?.sync_all()?;
        }
        let mut next_tick = Instant::now();
        mflux::run(
            &runtime,
            &mflux::Request {
                model: &model_path,
                prompt: &options.prompt,
                seed,
                width,
                height,
                steps,
                output: &output,
                input: snapshot.as_deref(),
            },
            &log,
            &signals.cancelled,
            |pid| store.record_worker(job_id, pid),
            || {
                if let Some(queue) = &queued
                    && Instant::now() >= next_tick
                {
                    if store.service_tick(job_id, &queue.server_id)?
                        || queue.shutdown.load(Ordering::Relaxed)
                    {
                        signals.cancelled.store(true, Ordering::Relaxed);
                    }
                    next_tick = Instant::now() + std::time::Duration::from_millis(500);
                }
                Ok(())
            },
        )?;
        validate_png(&output, width, height)?;
        let bytes = fs::read(&output)?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        fs::File::open(&output)?.sync_all()?;
        let mut result = json!({"job_id":job_id,"operation":kind,"input":input_metadata,"status":"succeeded","model":model.id,"model_revision":model.revision,"seed":seed,"width":width,"height":height,"steps":steps,
            "elapsed_ms":started.elapsed().as_millis(),"asset":{"id":asset_id,"path":output,"media_type":"image/png","sha256":hash},"export_path":null,"log_path":log});
        store.complete_image_job(
            job_id,
            &asset_id,
            output.to_str().context("asset path is not UTF-8")?,
            &hash,
            &result,
        )?;
        if queued.is_some() {
            return Ok(result);
        }
        let exported = if let Some(export) = &export {
            export_png(&bytes, export).map(|()| export.clone())
        } else {
            let (directory, stem) = default_export
                .as_ref()
                .context("default export destination missing")?;
            export_default(&bytes, directory, stem)
        };
        match exported {
            Ok(path) => {
                result["export_path"] = json!(path);
                store.update_job_result(job_id, &result)?;
            }
            Err(error) => {
                return Err(problem(
                    "export_failed",
                    "The image was generated but could not be exported.",
                    "Use the preserved asset path or choose a writable unused output path.",
                    json!({"job_id":job_id,"asset_path":output,"causes":error.chain().map(ToString::to_string).collect::<Vec<_>>()}),
                ));
            }
        }
        Ok(result)
    })();
    match operation {
        Ok(result) => Ok(result),
        Err(error) => {
            let error = if queued
                .as_ref()
                .is_some_and(|q| q.shutdown.load(Ordering::Relaxed))
                && error
                    .downcast_ref::<ModelError>()
                    .is_some_and(|e| e.code == "generation_cancelled")
                && !store
                    .job_info(job_id)?
                    .is_some_and(|j| j["request"]["queue"]["cancel_requested"] == true)
            {
                problem(
                    "generation_interrupted",
                    "The service stopped during execution.",
                    "Inspect the task and resubmit explicitly if needed; queued tasks remain preserved.",
                    json!({"job_id":job_id}),
                )
            } else {
                error
            };
            let mut response = crate::cli::output::operation_error(&error)["error"].clone();
            response["details"]["job_id"] = json!(job_id);
            let status = if response["code"] == "generation_cancelled" {
                "cancelled"
            } else if response["code"] == "generation_interrupted" {
                "interrupted"
            } else {
                "failed"
            };
            store.fail_image_job(job_id, status, &response)?;
            if let Some(known) = error.downcast_ref::<ModelError>() {
                return Err(ModelError {
                    code: known.code,
                    message: known.message,
                    retryable: known.retryable,
                    next_step: known.next_step,
                    details: response["details"].clone(),
                }
                .into());
            }
            Err(problem(
                "generation_failed",
                "Image generation could not be completed.",
                "Inspect job info and the job log before retrying.",
                json!({"job_id":job_id,"log_path":log,"causes":response}),
            ))
        }
    }
}

fn validate_png(path: &Path, width: u32, height: u32) -> Result<()> {
    let invalid = || {
        problem(
            "invalid_image_output",
            "The generation component did not produce a valid PNG with the requested dimensions.",
            "Inspect the job log and component compatibility.",
            json!({}),
        )
    };
    let file = fs::File::open(path).map_err(|_| invalid())?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .map_err(|_| invalid())?;
    if reader.info().width != width || reader.info().height != height {
        return Err(invalid());
    }
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= 32 * 1024 * 1024)
        .ok_or_else(invalid)?;
    reader
        .next_frame(&mut vec![0u8; size])
        .map_err(|_| invalid())?;
    Ok(())
}
pub(crate) fn export_default(bytes: &[u8], directory: &Path, stem: &str) -> Result<PathBuf> {
    for number in 1..=10000 {
        let filename = if number == 1 {
            format!("{stem}.png")
        } else {
            format!("{stem}_{number}.png")
        };
        let path = directory.join(filename);
        match export_png(bytes, &path) {
            Ok(()) => return Ok(path),
            Err(error)
                if error
                    .downcast_ref::<tempfile::PersistError>()
                    .is_some_and(|e| e.error.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    anyhow::bail!("too many output filename collisions")
}

pub(crate) fn export_png(bytes: &[u8], path: &Path) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("export path has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(path)?;
    Ok(())
}

#[cfg(test)]
mod export_tests {
    use super::*;
    #[test]
    fn timestamp_collisions_append_a_suffix_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let stem = "image_1105_1729";
        let first = export_default(b"first", root.path(), stem).unwrap();
        let second = export_default(b"second", root.path(), stem).unwrap();
        assert_eq!(first.file_name().unwrap(), "image_1105_1729.png");
        assert_eq!(second.file_name().unwrap(), "image_1105_1729_2.png");
        assert_eq!(fs::read(first).unwrap(), b"first");
    }
    #[test]
    fn concurrent_exports_choose_unique_names() {
        let root = tempfile::tempdir().unwrap();
        let handles = (0..4)
            .map(|_| {
                let path = root.path().to_owned();
                std::thread::spawn(move || {
                    export_default(b"image", &path, "image_1105_1729").unwrap()
                })
            })
            .collect::<Vec<_>>();
        let paths = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(paths.len(), 4);
        for path in paths {
            assert_eq!(fs::read(path).unwrap(), b"image");
        }
    }
}
