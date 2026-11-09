#![cfg(target_os = "macos")]
//! Explicit real launchd smoke check; leaves the user's ordinary XDG roots unchanged.
use serde_json::Value;
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
fn command(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gendock"));
    cmd.env("HOME", root)
        .env_remove("GENDOCK_URL")
        .env_remove("GENDOCK_TOKEN");
    for key in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
    ] {
        cmd.env_remove(key);
    }
    cmd
}
fn run(root: &Path, args: &[&str]) -> Value {
    let output = command(root).args(args).output().unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{value}");
    value["data"].clone()
}
struct Cleanup(std::path::PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = command(&self.0).args(["server", "unregister"]).output();
    }
}
#[test]
#[ignore = "requires an interactive macOS launchd session and no existing gendockd registration"]
fn register_launchd_start_status_and_unregister() {
    let uid = Command::new("/usr/bin/id").arg("-u").output().unwrap();
    let target = format!(
        "gui/{}/gendockd",
        String::from_utf8(uid.stdout).unwrap().trim()
    );
    assert!(
        !Command::new("/bin/launchctl")
            .args(["print", &target])
            .output()
            .unwrap()
            .status
            .success(),
        "An existing gendockd job must not be replaced by a test"
    );
    let root = tempfile::tempdir().unwrap();
    let _cleanup = Cleanup(root.path().to_owned());
    let registered = run(
        root.path(),
        &["server", "start", "--register", "--port", "0"],
    );
    assert_eq!(registered["service"], "gendockd");
    assert_eq!(registered["registered"], true);
    assert_eq!(registered["starts_at"], "user_login");
    assert!(
        Command::new("/bin/launchctl")
            .args(["print", &target])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(run(root.path(), &["server", "status"])["running"], true);
    let output = Command::new("/usr/bin/plutil")
        .args(["-lint", registered["plist"].as_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        run(root.path(), &["server", "unregister"])["registered"],
        false
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while root
        .path()
        .join(".local/state/gendock/server.json")
        .exists()
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !Command::new("/bin/launchctl")
            .args(["print", &target])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !root
            .path()
            .join("Library/LaunchAgents/gendockd.plist")
            .exists()
    );
}
