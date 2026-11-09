#![cfg(unix)]
//! Opt-in real-model acceptance. No downloads; user models/runtime remain read-only.
use gendock::{image::DEFAULT_MODEL, paths::Paths, store::Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn command(root: &Path) -> Command {
    std::fs::create_dir_all(root).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gendock"));
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
fn run(mut cmd: Command, args: &[&str]) -> Value {
    let output = cmd.args(args).output().unwrap();
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("CLI must return JSON");
    assert!(
        output.status.success() && value["ok"] == true,
        "CLI failed: {value}"
    );
    value["data"].clone()
}
struct Service(PathBuf);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = command(&self.0).args(["server", "stop"]).output();
    }
}
fn pixel_hash(path: &Path) -> String {
    let mut reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    assert_eq!((reader.info().width, reader.info().height), (512, 512));
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    format!("{:x}", Sha256::digest(&pixels[..frame.buffer_size()]))
}
#[test]
#[ignore = "requires an installed FLUX.2 Klein model and registered private generate/edit components"]
fn real_remote_generate_edit_wait_fetch() {
    let existing = Paths::discover().unwrap();
    let model = std::env::var_os("GENDOCK_E2E_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let conn = rusqlite::Connection::open_with_flags(
                existing.database(),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .expect("prepare the model first: gendock model download --model flux2-klein-4b-q4");
            PathBuf::from(
                conn.query_row(
                    "SELECT path FROM models WHERE name=?1",
                    [DEFAULT_MODEL],
                    |r| r.get::<_, String>(0),
                )
                .expect("the model must already be registered; this test never downloads it"),
            )
        });
    let runtime = std::fs::read(existing.data.join("runtimes/image.json"))
        .expect("a registered private generate/edit runtime is required; no system Python is used");
    let root = tempfile::Builder::new()
        .prefix("gendock-real-e2e-")
        .tempdir()
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap();
    let host = root.join("host");
    std::fs::create_dir_all(&host).unwrap();
    let paths = Paths::resolve(&host, |_| None);
    Store::open(&paths.database())
        .unwrap()
        .add_local_model(DEFAULT_MODEL, &model, "mflux", "image", None)
        .unwrap();
    std::fs::create_dir_all(paths.data.join("runtimes")).unwrap();
    std::fs::write(paths.data.join("runtimes/image.json"), runtime).unwrap();
    let started = run(
        command(&host),
        &["server", "start", "--daemon", "--port", "0"],
    );
    let _service = Service(host.clone());
    let token = run(command(&host), &["server", "token", "--name", "acceptance"])["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let url = started["url"].as_str().unwrap();
    let client = root.join("client");
    let client_command = || {
        let mut cmd = command(&client);
        cmd.env("GENDOCK_URL", url).env("GENDOCK_TOKEN", &token);
        cmd
    };
    let generated = run(
        client_command(),
        &[
            "image",
            "generate",
            "--prompt",
            "A blue ceramic cup on a plain white table, studio photograph, no text",
            "--model",
            DEFAULT_MODEL,
            "--width",
            "512",
            "--height",
            "512",
            "--steps",
            "4",
            "--seed",
            "42",
            "--async",
        ],
    );
    let first = generated["job_id"].as_i64().unwrap().to_string();
    assert_eq!(
        run(
            client_command(),
            &["job", "wait", "--id", &first, "--timeout", "600"]
        )["status"],
        "succeeded"
    );
    let fetched = run(
        client_command(),
        &["job", "fetch", "--id", &first, "--out", "generated"],
    );
    let original = PathBuf::from(fetched["path"].as_str().unwrap());
    let original_bytes = std::fs::read(&original).unwrap();
    let edited = run(
        client_command(),
        &[
            "image",
            "edit",
            "--input",
            original.to_str().unwrap(),
            "--prompt",
            "Change the blue ceramic cup to green, preserving the table, composition and lighting",
            "--model",
            DEFAULT_MODEL,
            "--width",
            "512",
            "--height",
            "512",
            "--steps",
            "4",
            "--seed",
            "42",
            "--async",
        ],
    );
    let second = edited["job_id"].as_i64().unwrap().to_string();
    assert_eq!(
        run(
            client_command(),
            &["job", "wait", "--id", &second, "--timeout", "600"]
        )["status"],
        "succeeded"
    );
    let fetched = run(
        client_command(),
        &["job", "fetch", "--id", &second, "--out", "edited"],
    );
    let output = PathBuf::from(fetched["path"].as_str().unwrap());
    assert_eq!(
        std::fs::read(&original).unwrap(),
        original_bytes,
        "editing must preserve the original bytes"
    );
    let before = pixel_hash(&original);
    let after = pixel_hash(&output);
    assert_ne!(
        before, after,
        "edited image pixels must differ, not just PNG metadata"
    );
    let job = Store::open(&paths.database())
        .unwrap()
        .job_info(second.parse().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        job["request"]["input"]["sha256"],
        format!("{:x}", Sha256::digest(&original_bytes))
    );
    assert!(
        !client.join(".local/share/gendock/models").exists(),
        "remote clients must not install model weights"
    );
    let report = json!({"model":DEFAULT_MODEL,"generation_job":first,"editing_job":second,"generated":original,"edited":output,"original_preserved":true,"pixel_content_changed":true,"generation_pixel_sha256":before,"editing_pixel_sha256":after,"scope":"One real Apple Silicon recipe; image semantics still need visual review."});
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("Real E2E artifacts: {}", root.display());
}
