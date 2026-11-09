use super::ImageRuntime;
use crate::models::ModelError;
use anyhow::Result;
use serde_json::json;
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub struct Request<'a> {
    pub model: &'a Path,
    pub prompt: &'a str,
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub output: &'a Path,
    pub input: Option<&'a Path>,
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            // The child is still unreaped, so its PID/process group cannot be reused.
            #[cfg(unix)]
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &format!("-{}", self.0.id())])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

pub struct Signals {
    pub cancelled: Arc<AtomicBool>,
    ids: Vec<signal_hook::SigId>,
}
impl Signals {
    pub fn install() -> Result<Self> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut guard = Self {
            cancelled,
            ids: Vec::new(),
        };
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            guard.ids.push(signal_hook::flag::register(
                signal,
                guard.cancelled.clone(),
            )?);
        }
        Ok(guard)
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        for id in self.ids.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

pub fn run(
    runtime: &ImageRuntime,
    request: &Request<'_>,
    log: &Path,
    cancelled: &AtomicBool,
    on_spawn: impl FnOnce(u32) -> Result<()>,
    mut on_tick: impl FnMut() -> Result<()>,
) -> Result<()> {
    let stdout = std::fs::File::create(log)?;
    let stderr = stdout.try_clone()?;
    let mut command = Command::new(runtime.worker(request.input.is_some())?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .args(["--model"])
        .arg(request.model)
        .args(["--base-model", "flux2-klein-4b"]);
    if let Some(input) = request.input {
        command.arg("--image-paths").arg(input);
    }
    command
        .args(["--prompt", request.prompt])
        .args([
            "--seed",
            &request.seed.to_string(),
            "--steps",
            &request.steps.to_string(),
            "--width",
            &request.width.to_string(),
            "--height",
            &request.height.to_string(),
            "--output",
        ])
        .arg(request.output)
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1")
        .env("PYTHONNOUSERSITE", "1")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env_remove("VIRTUAL_ENV")
        .env_remove("CONDA_PREFIX")
        .env_remove("HF_TOKEN")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    let mut child = ChildGuard(command.spawn()?);
    on_spawn(child.0.id())?;
    loop {
        on_tick()?;
        if cancelled.load(Ordering::Relaxed) {
            return Err(ModelError {code:"generation_cancelled",message:"Image generation was cancelled.",retryable:false,next_step:"Submit a new request when ready. Partial files and logs are retained for inspection.",details:json!({"log_path":log})}.into());
        }
        if let Some(status) = child.0.try_wait()? {
            if status.success() {
                return Ok(());
            }
            return Err(ModelError {code:"generation_failed",message:"The image generation component failed.",retryable:false,next_step:"Inspect the job log, verify model/component compatibility and available memory, then correct the issue before retrying.",details:json!({"exit_code":status.code(),"log_path":log})}.into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
