pub mod download;
pub mod hub;
pub mod manifest;

fn user_origin() -> String {
    "user".into()
}
fn other_type() -> String {
    "other".into()
}
fn active_status() -> String {
    "active".into()
}

pub const BUILTIN_CATALOG_REVISION: i64 = 2;

use crate::paths::Paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct File {
    pub path: String,
    pub size: u64,
    pub hash: String,
    pub algorithm: String,
    /// A direct file URL (R2, CDN, etc.). None uses the pinned HF resolver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    #[serde(default = "user_origin")]
    pub origin: String,
    #[serde(default)]
    pub family: String,
    #[serde(default = "other_type", rename = "type")]
    pub model_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage_url: Option<String>,
    #[serde(default = "active_status")]
    pub status: String,
    #[serde(default)]
    pub recommended: bool,
    #[serde(default)]
    pub replaced_by: Option<String>,
    #[serde(default)]
    pub deprecation_reason: Option<String>,
    #[serde(default)]
    pub repository: String,
    pub revision: String,
    pub license: String,
    pub backend: String,
    #[serde(default)]
    pub tasks: Vec<String>,
    #[serde(default)]
    pub minimum_memory_gib: u64,
    #[serde(default)]
    pub files: Vec<File>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            origin: user_origin(),
            family: String::new(),
            model_type: other_type(),
            description: String::new(),
            homepage_url: None,
            status: active_status(),
            recommended: false,
            replaced_by: None,
            deprecation_reason: None,
            repository: String::new(),
            revision: String::new(),
            license: String::new(),
            backend: String::new(),
            tasks: Vec::new(),
            minimum_memory_gib: 0,
            files: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct ModelError {
    pub code: &'static str,
    pub message: &'static str,
    pub retryable: bool,
    pub next_step: &'static str,
    pub details: Value,
}
impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ModelError {}

pub fn catalog() -> Vec<Model> {
    serde_json::from_str(include_str!("../../catalog/models.json"))
        .expect("validated bundled catalog")
}

pub fn find(id: &str) -> Result<Model> {
    crate::store::Store::open(&Paths::discover()?.database())?.catalog_find(id)
}

#[derive(Serialize)]
pub struct Device {
    pub os: &'static str,
    pub architecture: &'static str,
    pub chip: Option<String>,
    pub os_version: Option<String>,
    pub total_memory_bytes: Option<u64>,
}
impl Device {
    pub fn detect() -> Self {
        fn read(command: &str, args: &[&str]) -> Option<String> {
            let output = std::process::Command::new(command)
                .args(args)
                .output()
                .ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        }
        Self {
            os: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            chip: if cfg!(target_os = "macos") {
                read("/usr/sbin/sysctl", &["-n", "machdep.cpu.brand_string"])
            } else {
                None
            },
            os_version: if cfg!(target_os = "macos") {
                read("/usr/bin/sw_vers", &["-productVersion"])
            } else {
                None
            },
            total_memory_bytes: if cfg!(target_os = "macos") {
                read("/usr/sbin/sysctl", &["-n", "hw.memsize"]).and_then(|v| v.parse().ok())
            } else {
                None
            },
        }
    }

    pub fn compatibility(&self, model: &Model) -> Value {
        let (status, reason) = if model.backend != "mflux" {
            (
                "unknown",
                "This backend has not been integrated or validated.",
            )
        } else if self.os != "macos" || self.architecture != "aarch64" {
            (
                "unsupported",
                "The planned mflux backend requires macOS on Apple Silicon.",
            )
        } else if model.minimum_memory_gib == 0 {
            (
                "unknown",
                "No minimum memory estimate is available for this model.",
            )
        } else if let Some(memory) = self.total_memory_bytes {
            if memory < model.minimum_memory_gib.saturating_mul(1024 * 1024 * 1024) {
                (
                    "unsupported",
                    "Total memory is below the catalog's conservative minimum estimate.",
                )
            } else {
                (
                    "candidate",
                    "Hardware passes initial estimates; runtime and generation have not been validated.",
                )
            }
        } else {
            ("unknown", "Unable to detect total memory.")
        };
        json!({"status":status,"reason":reason,"minimum_memory_gib_estimate":model.minimum_memory_gib,"runtime_ready":false})
    }
}

pub fn describe(
    model: &Model,
    device: &Device,
    paths: &Paths,
    registrations: &[crate::store::Model],
) -> Value {
    let registered = registrations.iter().find(|entry| entry.name == model.id);
    let root = registered
        .map(|entry| std::path::PathBuf::from(&entry.path))
        .unwrap_or_else(|| {
            paths
                .data
                .join("models")
                .join(&model.id)
                .join(&model.revision)
        });
    let execution_supported = crate::image::supports(model);
    let runtime_ready = execution_supported && crate::runtime::ImageRuntime::load(paths).is_ok();
    let files_available = registered.is_some()
        && !model.files.is_empty()
        && model.files.iter().all(|file| {
            root.join(&file.path)
                .metadata()
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() == file.size)
        });
    let mut compatibility = device.compatibility(model);
    compatibility["runtime_ready"] = json!(runtime_ready);
    let ready = runtime_ready
        && files_available
        && model.status != "retired"
        && compatibility["status"] == "candidate";
    let edit_runtime_ready = execution_supported
        && model.tasks.iter().any(|task| task == "image.edit")
        && crate::runtime::ImageRuntime::load_for(paths, true).is_ok();
    let mut ready_operations = Vec::new();
    if ready {
        ready_operations.push("image.generate");
    }
    if edit_runtime_ready
        && files_available
        && model.status != "retired"
        && compatibility["status"] == "candidate"
    {
        ready_operations.push("image.edit");
    }
    json!({"id":model.id,"name":model.name,"origin":model.origin,"type":model.model_type,"family":model.family,"description":model.description,
        "homepage_url":model.homepage_url,"status":model.status,"recommended":model.recommended,
        "replaced_by":model.replaced_by,"deprecation_reason":model.deprecation_reason,
        "download_urls":model.files.iter().map(|f|f.url.clone().unwrap_or_else(||format!("https://huggingface.co/{}/resolve/{}/{}",model.repository,model.revision,f.path))).collect::<Vec<_>>(),
        "downloadable":!model.files.is_empty() && model.status!="retired",
        "repository":model.repository,"revision":model.revision,
        "license":model.license,"license_url":model.homepage_url,
        "backend":model.backend,"tasks":model.tasks,"download_bytes":model.files.iter().map(|f|f.size).sum::<u64>(),
        "compatibility":compatibility,"installation":{"path":root,"registered":registered.is_some(),"ownership":registered.map(|r|r.ownership.as_str()),"manifest_present":root.join("gendock-manifest.json").is_file(),"integrity_checked":false},
        "ready_operations":ready_operations,"execution_supported":execution_supported,"files_available":files_available,"readiness_check":"configuration_and_file_sizes","inference_ready":ready})
}

pub fn search(query: &str) -> Result<Value> {
    let paths = Paths::discover()?;
    let store = crate::store::Store::open(&paths.database())?;
    let registrations = store.models()?;
    let device = Device::detect();
    let results = store
        .catalog_search(query)?
        .iter()
        .map(|model| describe(model, &device, &paths, &registrations))
        .collect::<Vec<_>>();
    Ok(json!({"source":"sqlite_catalog","query":query,"device":device,"models":results}))
}

pub fn info(id: &str) -> Result<Value> {
    let model = find(id)?;
    let device = Device::detect();
    let paths = Paths::discover()?;
    let registrations = crate::store::Store::read_models(&paths.database())?;
    let mut value = describe(&model, &device, &paths, &registrations);
    value["device"] = serde_json::to_value(&device)?;
    value["files"] = serde_json::to_value(&model.files)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_pinned_and_valid() {
        let models = catalog();
        assert!(!models.is_empty());
        for model in models {
            assert_eq!(model.revision.len(), 40);
            assert!(!model.files.is_empty());
            for file in model.files {
                hub::validate_path(&file.path).unwrap();
                assert!(file.size > 0);
            }
        }
    }
    #[test]
    fn compatibility_distinguishes_unknown_and_unsupported() {
        let mut device = Device {
            os: "macos",
            architecture: "aarch64",
            chip: None,
            os_version: None,
            total_memory_bytes: None,
        };
        let model = &catalog()[0];
        assert_eq!(device.compatibility(model)["status"], "unknown");
        device.total_memory_bytes = Some(1024);
        assert_eq!(device.compatibility(model)["status"], "unsupported");
        device.architecture = "x86_64";
        device.total_memory_bytes = Some(64 * 1024 * 1024 * 1024);
        assert_eq!(device.compatibility(model)["status"], "unsupported");
    }
}
