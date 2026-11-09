//! A task-scoped sleep assertion; dropping it always reaps our own helper.
use anyhow::Result;

pub struct SleepGuard(Option<std::process::Child>);
impl SleepGuard {
    pub fn acquire(enabled: bool) -> Result<Self> {
        if !enabled {
            return Ok(Self(None));
        }
        #[cfg(target_os = "macos")]
        {
            let child = std::process::Command::new("/usr/bin/caffeinate")
                .args(["-i", "-w", &std::process::id().to_string()])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            Ok(Self(Some(child)))
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(crate::models::ModelError {
                code: "power_management_unsupported",
                message: "Sleep prevention is only supported on macOS.",
                retryable: false,
                next_step: "Set power.prevent_sleep to false on this platform.",
                details: serde_json::json!({}),
            }
            .into())
        }
    }
}
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn task_assertion_is_reaped_on_drop() {
        let guard = SleepGuard::acquire(true).unwrap();
        let pid = guard.0.as_ref().unwrap().id();
        drop(guard);
        let status = std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success());
        assert!(SleepGuard::acquire(false).unwrap().0.is_none());
    }
}

impl Drop for SleepGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
