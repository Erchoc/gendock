//! Private execution components; never discover Python from the user's PATH.
pub mod mflux;

use crate::{models::ModelError, paths::Paths};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Deserialize, Serialize)]
pub struct ImageRuntime {
    pub adapter: String,
    pub version: String,
    pub executable: PathBuf,
    pub executable_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_executable: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_executable_sha256: Option<String>,
}
fn edit_unavailable() -> ModelError {
    ModelError {
        code: "runtime_not_ready",
        message: "The image editing component is not ready.",
        retryable: false,
        next_step: "Register or repair the private editing component; see docs/image-generation.md. No model download was started.",
        details: json!({"component":"image.edit"}),
    }
}

impl ImageRuntime {
    pub fn load_for(paths: &Paths, editing: bool) -> Result<Self> {
        let runtime = Self::load(paths)?;
        if editing {
            let executable = runtime.worker(true)?;
            let hash = runtime
                .edit_executable_sha256
                .as_deref()
                .ok_or_else(edit_unavailable)?;
            if !executable.is_absolute()
                || !executable.is_file()
                || format!(
                    "{:x}",
                    Sha256::digest(std::fs::read(executable).map_err(|_| edit_unavailable())?)
                ) != hash
            {
                return Err(edit_unavailable().into());
            }
        }
        Ok(runtime)
    }
    pub fn worker(&self, editing: bool) -> Result<&std::path::Path> {
        if editing {
            self.edit_executable
                .as_deref()
                .ok_or_else(|| edit_unavailable().into())
        } else {
            Ok(&self.executable)
        }
    }
    pub fn load(paths: &Paths) -> Result<Self> {
        let manifest = paths.data.join("runtimes/image.json");
        let unavailable = || ModelError {
            code: "runtime_not_ready",
            message: "The image generation component is not ready.",
            retryable: false,
            next_step: "The model files and execution component are separate. This development build needs a registered private image runtime; see docs/image-generation.md.",
            details: json!({"component":"image","runtime_manifest":manifest}),
        };
        let bytes = std::fs::read(&manifest).map_err(|_| unavailable())?;
        let runtime: Self = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        if runtime.adapter != "mflux"
            || runtime.version != "0.18.0"
            || !runtime.executable.is_absolute()
            || !runtime.executable.is_file()
        {
            return Err(unavailable().into());
        }
        let actual = format!(
            "{:x}",
            Sha256::digest(std::fs::read(&runtime.executable).map_err(|_| unavailable())?)
        );
        if actual != runtime.executable_sha256 {
            return Err(unavailable().into());
        }
        Ok(runtime)
    }
}
