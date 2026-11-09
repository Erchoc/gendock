use super::{Model, ModelError, hub};
use anyhow::Result;
use serde_json::json;

pub fn validate(model: &Model) -> Result<()> {
    fn fail(field: &str) -> anyhow::Error {
        ModelError {code:"invalid_model_manifest",message:"The model manifest is invalid.",retryable:false,
            next_step:"Inspect docs/models.md and correct the indicated field. Provide immutable revisions and file sizes/hashes.",details:json!({"field":field})}.into()
    }
    fn identifier(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 128
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    }
    if !identifier(&model.id) {
        return Err(fail("id"));
    }
    if model.name.trim().is_empty() {
        return Err(fail("name"));
    }
    if !identifier(&model.revision) {
        return Err(fail("revision"));
    }
    if !["text", "image", "video", "audio", "music", "other"].contains(&model.model_type.as_str()) {
        return Err(fail("type"));
    }
    if !["active", "deprecated", "retired"].contains(&model.status.as_str())
        || (model.status != "active" && model.recommended)
    {
        return Err(fail("status/recommended"));
    }
    if model.backend.trim().is_empty() {
        return Err(fail("backend"));
    }
    if !model.files.is_empty() && model.homepage_url.is_none() {
        return Err(fail("homepage_url"));
    }
    if let Some(homepage) = &model.homepage_url {
        hub::validate_url(homepage).map_err(|_| fail("homepage_url"))?;
    }
    let mut paths = std::collections::HashSet::new();
    let mut total = 0u64;
    for file in &model.files {
        hub::validate_path(&file.path).map_err(|_| fail("files.path"))?;
        if file.path == "gendock-manifest.json"
            || !paths.insert(file.path.clone())
            || file.size == 0
        {
            return Err(fail("files.path/size"));
        }
        total = total
            .checked_add(file.size)
            .ok_or_else(|| fail("files.size"))?;
        let digits = match file.algorithm.as_str() {
            "sha256" => 64,
            "git_sha1" => 40,
            _ => return Err(fail("files.algorithm")),
        };
        if file.hash.len() != digits
            || !file
                .hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(fail("files.hash"));
        }
        if let Some(url) = &file.url {
            hub::validate_url(url).map_err(|_| fail("files.url"))?;
        } else if model.repository.split('/').count() != 2
            || model
                .repository
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
            || model.revision.len() != 40
            || !model.revision.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(fail("repository/revision"));
        }
    }
    for path in &paths {
        if paths.contains(&format!("{path}.part"))
            || paths
                .iter()
                .any(|other| other.starts_with(&format!("{path}/")))
        {
            return Err(fail("files.path"));
        }
    }
    Ok(())
}
