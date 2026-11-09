#![cfg(unix)]
use serde_json::Value;
use std::{
    io::{BufRead, Read},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
fn command(root: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_gendock"));
    c.env("HOME", root)
        .env_remove("GENDOCK_URL")
        .env_remove("GENDOCK_TOKEN");
    for k in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
    ] {
        c.env_remove(k);
    }
    c
}
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn default_foreground_streams_colored_leveled_logs_without_success_json() {
    let root = tempfile::tempdir().unwrap();
    let mut child = Owned(
        command(root.path())
            .args(["server", "start", "--port", "0"])
            .env("CLICOLOR_FORCE", "1")
            .env_remove("NO_COLOR")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (send, recv) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if send.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let first = recv
        .recv_timeout(Duration::from_secs(15))
        .expect("foreground service did not report readiness");
    assert!(first.contains("\x1b[32mINFO"), "{first}");
    assert!(first.contains("gendockd server_started"));
    assert!(first.contains("http://127.0.0.1:"));
    assert!(first.contains("http://localhost:"));
    assert!(first.contains("lan_addresses"));
    assert!(first.contains("Loopback only"));
    assert!(child.0.try_wait().unwrap().is_none());
    let status = command(root.path())
        .args(["server", "status"])
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["data"]["running"], true);
    let paths = gendock::paths::Paths::resolve(root.path(), |_| None);
    let receipt = gendock::server::read_receipt(&paths).unwrap();
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let response = http
        .get(format!("{}/v1/status?token=never-log-query", receipt.url))
        .bearer_auth("never-log-auth")
        .send()
        .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(
        response.headers()["www-authenticate"],
        "Bearer realm=\"gendockd\""
    );
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let failure: Value = response.json().unwrap();
    assert_eq!(failure["error"]["code"], "unauthorized");
    let unknown = http
        .get(format!("{}/favicon.ico?token=never-log-query", receipt.url))
        .send()
        .unwrap();
    assert_eq!(unknown.status(), 401);
    let unknown_id = unknown.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let _: Value = unknown.json().unwrap();
    let admin = gendock::server::auth::token(&paths, "admin").unwrap();
    assert_eq!(
        http.get(format!("{}/unknown-route", receipt.url))
            .bearer_auth(&admin)
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        http.get(format!("{}/v1/jobs/999999", receipt.url))
            .bearer_auth(&admin)
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        http.post(format!("{}/v1/jobs", receipt.url))
            .bearer_auth(&admin)
            .json(&serde_json::json!({"unknown_field":"never-log-body"}))
            .send()
            .unwrap()
            .status(),
        400
    );
    let credential_path = paths.config.join("server-tokens.json");
    let credentials = std::fs::read(&credential_path).unwrap();
    std::fs::write(&credential_path, b"not-json").unwrap();
    let server_failure = http
        .get(format!("{}/v1/status", receipt.url))
        .bearer_auth(&admin)
        .send()
        .unwrap();
    std::fs::write(&credential_path, credentials).unwrap();
    assert_eq!(server_failure.status(), 500);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = std::fs::read_to_string(gendock::server::log_path(&paths)).unwrap();
        if text.contains("invalid_submission") && text.contains("\"status_code\":500") {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let stop = command(root.path())
        .args(["server", "stop"])
        .output()
        .unwrap();
    assert!(stop.status.success());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(exit) = child.0.try_wait().unwrap() {
            assert!(exit.success());
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(25));
    }
    reader.join().unwrap();
    let rest = recv.into_iter().collect::<Vec<_>>();
    assert!(rest.iter().any(|line| line.contains("server_stopped")));
    assert!(rest.iter().any(|line| line.contains("\x1b[33mWARN")
        && line.contains(&request_id)
        && line.contains("\"status_code\":401")));
    let persisted = std::fs::read_to_string(gendock::server::log_path(&paths)).unwrap();
    for secret in [
        "never-log-query",
        "never-log-auth",
        "never-log-body",
        admin.as_str(),
    ] {
        assert!(!persisted.contains(secret));
        assert!(rest.iter().all(|line| !line.contains(secret)));
    }
    let records = persisted
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let rejected = records
        .iter()
        .find(|row| row["data"]["request_id"] == request_id)
        .unwrap();
    let unknown = records
        .iter()
        .find(|row| row["data"]["request_id"] == unknown_id)
        .unwrap();
    assert_eq!(unknown["data"]["path"], "/favicon.ico");
    assert_eq!(unknown["data"]["status_code"], 401);
    assert!(records.iter().any(|row|row["data"]["path"]=="/unknown-route" && row["data"]["status_code"]==404));
    assert_eq!(rejected["level"], "WARN");
    assert_eq!(rejected["data"]["status_code"], 401);
    assert_eq!(rejected["data"]["error_code"], "unauthorized");
    assert_eq!(rejected["data"]["path"], "/v1/status");
    assert!(rejected["data"]["duration_ms"].is_number());
    assert!(rejected["data"]["peer"].is_string());
    assert!(records.iter().any(|row| row["data"]["status_code"] == 200
        && row["level"] == "INFO"
        && row["data"]["client"] == "admin"));
    assert!(
        records
            .iter()
            .any(|row| row["data"]["status_code"] == 500 && row["level"] == "ERROR")
    );
    assert!(records.iter().any(
        |row| row["data"]["status_code"] == 404 && row["data"]["error_code"] == "job_not_found"
    ));
    assert!(
        rest.iter()
            .all(|line| serde_json::from_str::<Value>(line).is_err())
    );
    let mut stderr = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.is_empty(), "{stderr}");
}
#[test]
fn occupied_port_returns_specific_json_in_foreground_and_daemon_modes() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    for daemon in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut cmd = command(root.path());
        cmd.args(["server", "start", "--port", &port]);
        if daemon {
            cmd.arg("--daemon");
        }
        let output = cmd.output().unwrap();
        assert!(!output.status.success());
        assert!(output.stderr.is_empty());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["error"]["code"], "server_bind_failed", "{error}");
        assert_eq!(
            error["error"]["details"]["port"],
            port.parse::<u16>().unwrap()
        );
        assert!(error["error"]["details"]["reason"].is_string());
    }
}
#[test]
fn daemon_and_register_are_exclusive_and_help_is_concise() {
    let root = tempfile::tempdir().unwrap();
    let output = command(root.path())
        .args(["server", "start", "--daemon", "--register"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "invalid_arguments");
    let help = command(root.path()).arg("--help").output().unwrap();
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("Music creation"));
    assert!(!text.contains("including generation, voice cloning, and covers"));
}

#[test]
fn bare_host_enables_lan_and_explicit_host_is_preserved() {
    for host in [None, Some("127.0.0.1")] {
        let root = tempfile::tempdir().unwrap();
        let mut cmd = command(root.path());
        cmd.args(["server", "start", "--daemon", "--host"]);
        if let Some(host) = host {
            cmd.arg(host);
        }
        cmd.args(["--port", "0"]);
        let output = cmd.output().unwrap();
        let started: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.status.success(), "{started}");
        let paths = gendock::paths::Paths::resolve(root.path(), |_| None);
        let receipt = gendock::server::read_receipt(&paths).unwrap();
        let stopped = command(root.path())
            .args(["server", "stop"])
            .output()
            .unwrap();
        assert!(stopped.status.success());
        assert_eq!(receipt.host, host.unwrap_or("0.0.0.0"));
        assert_eq!(started["data"]["addresses"]["local_accessible"], true);
        for address in started["data"]["addresses"]["lan_addresses"]
            .as_array()
            .unwrap()
        {
            assert_eq!(address["accessible"], host.is_none());
        }
    }
}
