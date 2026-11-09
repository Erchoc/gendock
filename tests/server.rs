#![cfg(unix)]
//! End-to-end checks run the real CLI, HTTP listener, SQLite scheduler and tiny private workers.
use gendock::{client::Connection, models::catalog, paths::Paths, store::Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    paths: Paths,
    url: String,
}
fn command(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gendock"));
    std::fs::create_dir_all(root).unwrap();
    cmd.current_dir(root)
        .env("HOME", root)
        .env_remove("GENDOCK_URL")
        .env_remove("GENDOCK_TOKEN");
    for key in [
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
    ] {
        cmd.env_remove(key);
    }
    cmd
}
fn parse(output: Output) -> Value {
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("one JSON result");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(output.status.success(), value["ok"] == true, "{value}");
    value
}
impl Fixture {
    fn new(limit: u32) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let paths = Paths::resolve(&root, |_| None);
        let store = Store::open(&paths.database()).unwrap();
        let mut model = catalog()
            .into_iter()
            .find(|m| m.id == gendock::image::DEFAULT_MODEL)
            .unwrap();
        let dir = paths.data.join("models/fixture");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("weights.bin"), b"abc").unwrap();
        model.files=vec![serde_json::from_value(json!({"path":"weights.bin","size":3,"algorithm":"sha256","hash":format!("{:x}",Sha256::digest(b"abc")),"url":"https://example.com/weights.bin"})).unwrap()];
        rusqlite::Connection::open(paths.database())
            .unwrap()
            .execute(
                "UPDATE model_catalog SET definition=?2 WHERE id=?1",
                rusqlite::params![model.id, serde_json::to_string(&model).unwrap()],
            )
            .unwrap();
        store.register_managed(&model.id, &dir, "mflux").unwrap();
        let png = root.join("fixture.png");
        let mut encoder = png::Encoder::new(std::fs::File::create(&png).unwrap(), 256, 256);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![100; 256 * 256 * 3])
            .unwrap();
        let runtime = paths.data.join("runtimes");
        std::fs::create_dir_all(&runtime).unwrap();
        let script = format!(
            "#!/bin/sh\nset -eu\nout=\nprompt=\ninput=\nwhile [ \"$#\" -gt 0 ]; do\ncase \"$1\" in\n--output) out=$2; shift;;\n--prompt) prompt=$2; shift;;\n--image-paths) input=$2; shift;;\nesac\nshift\ndone\nprintf 'start:%s\\n' \"$prompt\" >> '{}'\nif [ -n \"$input\" ]; then cp \"$input\" '{}'; printf '%s' \"$input\" > '{}'; fi\ncase \"$prompt\" in slow*) sleep 3;; *) sleep 0.05;; esac\ncp '{}' \"$out\"\nprintf 'end:%s\\n' \"$prompt\" >> '{}'\n",
            root.join("trace").display(),
            root.join("captured-input").display(),
            root.join("snapshot-path").display(),
            png.display(),
            root.join("trace").display()
        );
        let executable = runtime.join("worker");
        std::fs::write(&executable, &script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let hash = format!("{:x}", Sha256::digest(script.as_bytes()));
        std::fs::write(runtime.join("image.json"),serde_json::to_vec(&json!({"adapter":"mflux","version":"0.18.0","executable":executable,"executable_sha256":hash,"edit_executable":executable,"edit_executable_sha256":hash})).unwrap()).unwrap();
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(
            paths.config.join("config.toml"),
            format!("[server]\nmax_pending_per_user={limit}\n"),
        )
        .unwrap();
        let mut fixture = Self {
            _temp: temp,
            root,
            paths,
            url: String::new(),
        };
        fixture.start();
        fixture
    }
    fn local(&self, args: &[&str]) -> Value {
        parse(command(&self.root).args(args).output().unwrap())
    }
    fn start(&mut self) {
        let result = self.local(&["server", "start", "--daemon", "--port", "0"]);
        assert_eq!(result["ok"], true, "{result}");
        self.url = result["data"]["url"].as_str().unwrap().into();
    }
    fn token(&self, name: &str) -> String {
        self.local(&["server", "token", "--name", name])["data"]["token"]
            .as_str()
            .unwrap()
            .into()
    }
    fn client_command(&self, name: &str) -> Command {
        let mut cmd = command(&self.root.join(format!("client-{name}")));
        cmd.env("GENDOCK_URL", &self.url)
            .env("GENDOCK_TOKEN", self.token(name));
        cmd
    }
    fn client(&self, name: &str, args: &[&str]) -> Value {
        parse(self.client_command(name).args(args).output().unwrap())
    }
    fn connection(&self, name: &str) -> Connection {
        Connection::new(&self.url, self.token(name)).unwrap()
    }
    fn submit(&self, name: &str, prompt: &str) -> i64 {
        let r = self.client(
            name,
            &[
                "image", "generate", "--prompt", prompt, "--width", "256", "--height", "256",
                "--async",
            ],
        );
        assert_eq!(r["ok"], true, "{r}");
        r["data"]["job_id"].as_i64().unwrap()
    }
    fn info(&self, id: i64) -> Value {
        Store::open(&self.paths.database())
            .unwrap()
            .job_info(id)
            .unwrap()
            .unwrap()
    }
    fn started(&self, prompt: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !std::fs::read_to_string(self.root.join("trace"))
            .unwrap_or_default()
            .contains(&format!("start:{prompt}\n"))
        {
            assert!(
                Instant::now() < deadline,
                "worker did not start; logs: {}",
                self.local(&["server", "logs"])
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = command(&self.root).args(["server", "stop"]).output();
    }
}

#[test]
fn remote_generate_edit_wait_fetch_and_client_isolation() {
    let f = Fixture::new(20);
    let alice = f.connection("alice");
    let bob = f.connection("bob");
    let unauthorized = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("{}/v1/status", f.url))
        .send()
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    assert!(alice.shutdown().is_err());
    let generated = f.submit("alice", "blue cup");
    let waited = f.client(
        "alice",
        &[
            "job",
            "wait",
            "--id",
            &generated.to_string(),
            "--timeout",
            "20",
        ],
    );
    assert_eq!(waited["data"]["status"], "succeeded", "{waited}");
    assert!(bob.info(generated).is_err());
    assert!(bob.cancel(generated).is_err());
    assert!(bob.bytes(generated).is_err());
    let fetched = f.client(
        "alice",
        &[
            "job",
            "fetch",
            "--id",
            &generated.to_string(),
            "--out",
            "images",
        ],
    );
    assert_eq!(fetched["ok"], true, "{fetched}");
    let input = PathBuf::from(fetched["data"]["path"].as_str().unwrap());
    let original = std::fs::read(&input).unwrap();
    let again = f.client(
        "alice",
        &[
            "job",
            "fetch",
            "--id",
            &generated.to_string(),
            "--out",
            "images",
        ],
    );
    assert!(again["data"]["path"].as_str().unwrap().ends_with("_2.png"));
    let edited = f.client(
        "alice",
        &[
            "image",
            "edit",
            "--input",
            input.to_str().unwrap(),
            "--prompt",
            "green cup",
            "--width",
            "256",
            "--height",
            "256",
            "--async",
        ],
    );
    assert_eq!(edited["ok"], true, "{edited}");
    let id = edited["data"]["job_id"].as_i64().unwrap();
    let stream = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap()
        .get(format!("{}/v1/jobs/{id}/events", f.url))
        .bearer_auth(f.token("alice"))
        .send()
        .unwrap();
    assert_eq!(stream.headers()["content-type"], "text/event-stream");
    let frames = stream.text().unwrap();
    assert!(frames.contains("event: job\n"));
    assert!(frames.contains("\"status\":\"succeeded\""), "{frames}");
    assert_eq!(
        f.client("alice", &["job", "wait", "--id", &id.to_string()])["ok"],
        true
    );
    assert_eq!(
        f.client(
            "alice",
            &["job", "fetch", "--id", &id.to_string(), "--out", "edits"]
        )["ok"],
        true
    );
    assert_eq!(std::fs::read(&input).unwrap(), original);
    assert_eq!(
        std::fs::read(f.root.join("captured-input")).unwrap(),
        original
    );
    let snapshot = std::fs::read_to_string(f.root.join("snapshot-path")).unwrap();
    assert!(snapshot.starts_with(f.paths.data.join("assets").to_str().unwrap()));
    assert_ne!(snapshot, input.to_string_lossy());
    let public = alice.info(id).unwrap().to_string();
    assert!(!public.contains(f.root.to_str().unwrap()));
    assert!(
        !f.root
            .join("client-alice/.local/share/gendock/database")
            .exists()
    );
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let private_upload = alice.info(id).unwrap()["request"]["input_upload"].clone();
    let denied: Value = http
        .post(format!("{}/v1/jobs", f.url))
        .bearer_auth(f.token("bob"))
        .json(&json!({"kind":"image.edit","prompt":"steal input","input_upload":private_upload}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(denied["error"]["code"], "upload_not_found");
    for payload in [
        json!({"kind":"image.edit","prompt":"x","input":"/etc/passwd"}),
        json!({"kind":"image.edit","prompt":"x","input_upload":"../../etc/passwd"}),
    ] {
        let response: Value = http
            .post(format!("{}/v1/jobs", f.url))
            .bearer_auth(f.token("alice"))
            .json(&payload)
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(response["ok"], false);
    }
    let permissions = std::fs::metadata(f.paths.config.join("server-tokens.json"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(permissions & 0o077, 0);
    assert_eq!(
        f.local(&["server", "token", "--name", "../bad"])["error"]["code"],
        "invalid_client_name"
    );
    assert_eq!(f.local(&["server", "status"])["data"]["gpu_concurrency"], 1);
}

#[test]
fn twenty_two_remote_clients_run_fifo_with_one_gpu_worker() {
    let f = Fixture::new(20);
    let mut handles = Vec::new();
    for index in 0..22 {
        let name = format!("person{index}");
        let prompt = format!("client-{index}");
        let mut cmd = f.client_command(&name);
        handles.push(std::thread::spawn(move || {
            let result = parse(
                cmd.args([
                    "image", "generate", "--prompt", &prompt, "--width", "256", "--height", "256",
                    "--async",
                ])
                .output()
                .unwrap(),
            );
            assert_eq!(result["ok"], true, "{result}");
            (result["data"]["job_id"].as_i64().unwrap(), prompt)
        }));
    }
    let mut tasks = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    tasks.sort_by_key(|(id, _)| *id);
    let admin = f.connection("admin");
    for (id, _) in &tasks {
        assert_eq!(admin.wait(*id, 30).unwrap()["status"], "succeeded");
    }
    let trace = std::fs::read_to_string(f.root.join("trace")).unwrap();
    let expected = tasks
        .iter()
        .flat_map(|(_, p)| [format!("start:{p}"), format!("end:{p}")])
        .collect::<Vec<_>>();
    assert_eq!(trace.lines().collect::<Vec<_>>(), expected);
    assert_eq!(
        Store::open(&f.paths.database())
            .unwrap()
            .visible_jobs("admin", true)
            .unwrap()
            .len(),
        22
    );
}

#[test]
fn pending_limit_cancellation_and_graceful_restart() {
    let mut f = Fixture::new(2);
    let alice = f.connection("alice");
    let first = f.submit("alice", "slow-cancel");
    f.started("slow-cancel");
    let second = f.submit("alice", "cancel-before-start");
    let full = f.client(
        "alice",
        &[
            "image", "generate", "--prompt", "overflow", "--width", "256", "--height", "256",
            "--async",
        ],
    );
    assert_eq!(full["error"]["code"], "queue_full");
    assert_eq!(alice.cancel(second).unwrap()["status"], "cancelled");
    alice.cancel(first).unwrap();
    assert_eq!(alice.wait(first, 15).unwrap()["status"], "cancelled");
    assert!(
        !std::fs::read_to_string(f.root.join("trace"))
            .unwrap()
            .contains("start:cancel-before-start")
    );
    let third = f.submit("alice", "slow-shutdown");
    f.started("slow-shutdown");
    let fourth = f.submit("alice", "retained-after-restart");
    assert_eq!(f.local(&["server", "stop"])["ok"], true);
    assert_eq!(f.info(third)["status"], "interrupted");
    assert_eq!(f.info(fourth)["status"], "queued");
    f.start();
    assert_eq!(
        f.connection("alice").wait(fourth, 20).unwrap()["status"],
        "succeeded"
    );
    assert_eq!(f.info(first)["status"], "cancelled");
}

#[test]
fn crash_keeps_pending_jobs_and_refuses_to_overlap_surviving_worker() {
    let mut f = Fixture::new(20);
    let first = f.submit("alice", "slow-orphan");
    f.started("slow-orphan");
    let second = f.submit("alice", "after-orphan");
    let receipt = gendock::server::read_receipt(&f.paths).unwrap();
    assert!(
        Command::new("/bin/kill")
            .args(["-KILL", &receipt.pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(f.paths.state.join("server.lock"))
            .unwrap();
        if fs2::FileExt::try_lock_exclusive(&lock).is_ok() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(25));
    }
    f.start();
    assert_eq!(
        f.connection("alice").wait(second, 25).unwrap()["status"],
        "succeeded"
    );
    assert_eq!(f.info(first)["status"], "interrupted");
    let trace = std::fs::read_to_string(f.root.join("trace")).unwrap();
    assert!(trace.find("end:slow-orphan").unwrap() < trace.find("start:after-orphan").unwrap());
    assert_eq!(trace.matches("start:slow-orphan").count(), 1);
}

#[test]
fn sse_delivers_live_updates_before_completion_and_replays_current_state() {
    use std::io::BufRead;
    let f = Fixture::new(20);
    let id = f.submit("alice", "slow-live-events");
    f.started("slow-live-events");
    let response = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap()
        .get(format!("{}/v1/jobs/{id}/events", f.url))
        .bearer_auth(f.token("alice"))
        .send()
        .unwrap();
    let mut reader = std::io::BufReader::new(response);
    let mut line = String::new();
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if let Some(data) = line.strip_prefix("data: ") {
            let frame: Value = serde_json::from_str(data).unwrap();
            assert_eq!(frame["data"]["status"], "running");
            break;
        }
    }
    f.connection("alice").cancel(id).unwrap();
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut reader, &mut rest).unwrap();
    assert!(rest.contains("\"status\":\"cancelled\""), "{rest}");
    let replay = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .get(format!("{}/v1/jobs/{id}/events", f.url))
        .bearer_auth(f.token("alice"))
        .header("Last-Event-ID", format!("{id}-running"))
        .send()
        .unwrap()
        .text()
        .unwrap();
    assert!(replay.contains("\"status\":\"cancelled\""));
}

#[test]
fn synchronous_remote_export_conflict_keeps_completed_asset_recoverable() {
    let f = Fixture::new(20);
    let mut cmd = f.client_command("alice");
    let destination = f.root.join("client-alice/export.png");
    let handle = std::thread::spawn(move || {
        parse(
            cmd.args([
                "image",
                "generate",
                "--prompt",
                "slow-export-race",
                "--width",
                "256",
                "--height",
                "256",
                "--out",
                "export.png",
            ])
            .output()
            .unwrap(),
        )
    });
    f.started("slow-export-race");
    std::fs::write(&destination, b"other writer").unwrap();
    let result = handle.join().unwrap();
    assert_eq!(result["error"]["code"], "export_failed", "{result}");
    let id = result["error"]["details"]["job_id"].as_i64().unwrap();
    assert_eq!(f.info(id)["status"], "succeeded");
    assert_eq!(std::fs::read(destination).unwrap(), b"other writer");
    assert_eq!(
        f.client(
            "alice",
            &["job", "fetch", "--id", &id.to_string(), "--out", "recovery"]
        )["ok"],
        true
    );
}
