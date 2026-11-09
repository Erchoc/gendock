#![cfg(unix)]
use gendock::{models::catalog, paths::Paths, store::Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    paths: Paths,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let paths = Paths::resolve(&root, |_| None);
        let store = Store::open(&paths.database()).unwrap();
        let mut model = catalog()
            .into_iter()
            .find(|m| m.id == gendock::image::DEFAULT_MODEL)
            .unwrap();
        let directory = paths
            .data
            .join("models")
            .join(&model.id)
            .join(&model.revision);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("weights.bin"), b"abc").unwrap();
        model.files=vec![serde_json::from_value(json!({"path":"weights.bin","size":3,"hash":format!("{:x}",Sha256::digest(b"abc")),"algorithm":"sha256","url":"https://example.com/weights.bin"})).unwrap()];
        let conn = rusqlite::Connection::open(paths.database()).unwrap();
        conn.execute(
            "UPDATE model_catalog SET definition=?2 WHERE id=?1",
            rusqlite::params![model.id, serde_json::to_string(&model).unwrap()],
        )
        .unwrap();
        store
            .register_managed(&model.id, &directory, "mflux")
            .unwrap();
        let png = root.join("fixture.png");
        let mut encoder = png::Encoder::new(std::fs::File::create(&png).unwrap(), 256, 256);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![128u8; 256 * 256 * 3])
            .unwrap();
        let runtime = paths.data.join("runtimes");
        std::fs::create_dir_all(&runtime).unwrap();
        let executable = runtime.join("fixture-worker");
        let action = match mode {
            "failure" => "echo worker-failed; exit 7".to_owned(),
            "invalid" => "printf not-a-png > \"$out\"".to_owned(),
            "wait" => format!("touch '{}'; sleep 5", root.join("started").display()),
            "export-conflict" => format!(
                "cp '{}' \"$out\"; printf racer > '{}'",
                png.display(),
                root.join("export.png").display()
            ),
            _ => format!("cp '{}' \"$out\"", png.display()),
        };
        let script = format!(
            "#!/bin/sh\n[ \"$HF_HUB_OFFLINE\" = 1 ] || exit 9\n[ \"$PYTHONNOUSERSITE\" = 1 ] || exit 9\n[ -z \"${{PYTHONPATH+x}}\" ] || exit 9\nout=\nwhile [ \"$#\" -gt 0 ]; do\ncase \"$1\" in\n--output) out=$2; shift;;\n--prompt) printf '%s' \"$2\" > '{}'; shift;;\n--image-paths) printf '%s' \"$2\" > '{}'; cp \"$2\" '{}'; shift;;\nesac\nshift\ndone\necho worker-progress\n{action}\n",
            root.join("prompt.txt").display(),
            root.join("input-path.txt").display(),
            root.join("captured-input.bin").display()
        );
        std::fs::write(&executable, &script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let edit_executable = runtime.join("fixture-edit-worker");
        let edit_script = format!(
            "#!/bin/sh\nprintf edit > '{}'\n{}",
            root.join("mode.txt").display(),
            script.strip_prefix("#!/bin/sh\n").unwrap()
        );
        std::fs::write(&edit_executable, &edit_script).unwrap();
        std::fs::set_permissions(&edit_executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(runtime.join("image.json"),serde_json::to_vec(&json!({"adapter":"mflux","version":"0.18.0","executable":executable,"executable_sha256":format!("{:x}",Sha256::digest(script.as_bytes())),"edit_executable":edit_executable,"edit_executable_sha256":format!("{:x}",Sha256::digest(edit_script.as_bytes()))})).unwrap()).unwrap();
        Self {
            _temp: temp,
            root,
            paths,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_gendock"));
        command
            .current_dir(&self.root)
            .env("HOME", &self.root)
            .env_remove("GENDOCK_URL")
            .env_remove("GENDOCK_TOKEN")
            .env("PYTHONPATH", "must-be-removed");
        for key in [
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
        ] {
            command.env_remove(key);
        }
        command
    }
    fn run(&self, extra: &[&str]) -> Output {
        self.command()
            .args([
                "image", "generate", "--prompt", "sea", "--width", "256", "--height", "256",
            ])
            .args(extra)
            .output()
            .unwrap()
    }
    fn run_edit(&self, extra: &[&str]) -> Output {
        self.command()
            .args(["image", "edit", "--input"])
            .arg(self.root.join("fixture.png"))
            .args([
                "--prompt",
                "change the cup to green",
                "--width",
                "256",
                "--height",
                "256",
            ])
            .args(extra)
            .output()
            .unwrap()
    }
    fn write_defaults(&self, generate: &str, edit: &str) {
        std::fs::create_dir_all(&self.paths.config).unwrap();
        std::fs::write(
            self.paths.config.join("config.toml"),
            format!("[defaults]\nimage_generate = '{generate}'\nimage_edit = '{edit}'\n"),
        )
        .unwrap();
    }
    fn parse(&self, out: &Output) -> Value {
        assert!(
            out.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
}

#[test]
fn editing_uses_the_edit_component_and_preserves_source_and_snapshot() {
    let fixture = Fixture::new("success");
    let source = fixture.root.join("fixture.png");
    let original = std::fs::read(&source).unwrap();
    let out = fixture.run_edit(&[]);
    let value = fixture.parse(&out);
    assert!(out.status.success(), "{value}");
    assert_eq!(value["data"]["operation"], "image.edit");
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("mode.txt")).unwrap(),
        "edit"
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
    let refused = fixture.parse(&fixture.run_edit(&["--out", source.to_str().unwrap()]));
    assert_eq!(refused["error"]["code"], "output_exists");
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(
        std::fs::read(fixture.root.join("captured-input.bin")).unwrap(),
        original
    );
    let snapshot = PathBuf::from(value["data"]["input"]["snapshot_path"].as_str().unwrap());
    assert_ne!(snapshot, source);
    assert_eq!(std::fs::read(&snapshot).unwrap(), original);
    let job = Store::open(&fixture.paths.database())
        .unwrap()
        .job_info(value["data"]["job_id"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(job["kind"], "image.edit");
    assert_eq!(job["request"]["model_selection"], "config");
    assert_eq!(
        job["request"]["input"]["snapshot_path"],
        value["data"]["input"]["snapshot_path"]
    );
    assert!(
        PathBuf::from(value["data"]["export_path"].as_str().unwrap()).starts_with(&fixture.root)
    );
}

#[test]
fn configured_defaults_are_separate_and_explicit_model_overrides_them() {
    let fixture = Fixture::new("success");
    fixture.write_defaults("missing-generate", "missing-edit");
    let generate = fixture.parse(&fixture.run(&[]));
    assert_eq!(generate["error"]["code"], "model_not_found");
    assert_eq!(generate["error"]["details"]["model"], "missing-generate");
    let edit = fixture.parse(&fixture.run_edit(&[]));
    assert_eq!(edit["error"]["details"]["model"], "missing-edit");
    assert!(
        fixture
            .run(&["--model", gendock::image::DEFAULT_MODEL])
            .status
            .success()
    );
    assert!(
        fixture
            .run_edit(&["--model", gendock::image::DEFAULT_MODEL])
            .status
            .success()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.paths.config.join("config.toml")).unwrap(),
        "[defaults]\nimage_generate = 'missing-generate'\nimage_edit = 'missing-edit'\n"
    );
}

#[test]
fn missing_default_models_report_download_actions_without_starting_a_job() {
    let fixture = Fixture::new("success");
    let store = Store::open(&fixture.paths.database()).unwrap();
    store.remove_model(gendock::image::DEFAULT_MODEL).unwrap();
    for out in [fixture.run(&[]), fixture.run_edit(&[])] {
        let value = fixture.parse(&out);
        assert_eq!(value["error"]["code"], "model_not_installed");
        assert_eq!(
            value["error"]["next_steps"][0]["argv"],
            json!([
                "gendock",
                "model",
                "download",
                "--model",
                gendock::image::DEFAULT_MODEL
            ])
        );
    }
    assert!(store.jobs().unwrap().is_empty());
    assert!(!fixture.root.join("prompt.txt").exists());
    // A registration alone is also insufficient when a required file is missing.
    let directory = fixture
        .paths
        .data
        .join("models")
        .join(gendock::image::DEFAULT_MODEL)
        .join(
            catalog()
                .into_iter()
                .find(|m| m.id == gendock::image::DEFAULT_MODEL)
                .unwrap()
                .revision,
        );
    store
        .register_managed(gendock::image::DEFAULT_MODEL, &directory, "mflux")
        .unwrap();
    std::fs::remove_file(directory.join("weights.bin")).unwrap();
    assert_eq!(
        fixture.parse(&fixture.run_edit(&[]))["error"]["code"],
        "model_files_incomplete"
    );
    assert!(store.jobs().unwrap().is_empty());
}

#[test]
fn input_validation_and_missing_edit_component_fail_before_execution() {
    let fixture = Fixture::new("success");
    let missing = fixture
        .command()
        .args([
            "image",
            "edit",
            "--input",
            "missing.png",
            "--prompt",
            "edit",
        ])
        .output()
        .unwrap();
    assert_eq!(fixture.parse(&missing)["error"]["code"], "input_not_found");
    let invalid = fixture.root.join("invalid.png");
    std::fs::write(&invalid, b"not an image").unwrap();
    let out = fixture
        .command()
        .args(["image", "edit", "--input"])
        .arg(&invalid)
        .args(["--prompt", "edit"])
        .output()
        .unwrap();
    assert_eq!(fixture.parse(&out)["error"]["code"], "invalid_input_image");
    let path = fixture.paths.data.join("runtimes/image.json");
    let mut runtime: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    runtime.as_object_mut().unwrap().remove("edit_executable");
    std::fs::write(path, serde_json::to_vec(&runtime).unwrap()).unwrap();
    assert_eq!(
        fixture.parse(&fixture.run_edit(&[]))["error"]["code"],
        "runtime_not_ready"
    );
    assert!(
        Store::open(&fixture.paths.database())
            .unwrap()
            .jobs()
            .unwrap()
            .is_empty()
    );
    assert!(!fixture.root.join("prompt.txt").exists());
}

#[test]
fn editing_accepts_jpeg_and_webp_inputs() {
    for (format, extension) in [
        (image::ImageFormat::Jpeg, "jpg"),
        (image::ImageFormat::WebP, "webp"),
    ] {
        let fixture = Fixture::new("success");
        let source = fixture.root.join(format!("source.{extension}"));
        image::DynamicImage::new_rgb8(256, 256)
            .save_with_format(&source, format)
            .unwrap();
        let original = std::fs::read(&source).unwrap();
        let out = fixture
            .command()
            .args(["image", "edit", "--input"])
            .arg(&source)
            .args(["--prompt", "edit", "--width", "256", "--height", "256"])
            .output()
            .unwrap();
        let value = fixture.parse(&out);
        assert!(out.status.success(), "{value}");
        assert_eq!(
            std::fs::read(fixture.root.join("captured-input.bin")).unwrap(),
            original
        );
        assert_eq!(std::fs::read(source).unwrap(), original);
    }
}

#[test]
fn configured_export_directories_and_explicit_paths_are_respected() {
    let fixture = Fixture::new("success");
    let generated = fixture.root.join("synced images");
    let edited = fixture.root.join("synced edits");
    std::fs::create_dir_all(&fixture.paths.config).unwrap();
    std::fs::write(
        fixture.paths.config.join("config.toml"),
        format!(
            "[output]\nimage = {:?}\nimage_edit = {:?}\n",
            generated.to_str().unwrap(),
            edited.to_str().unwrap()
        ),
    )
    .unwrap();
    let value = fixture.parse(&fixture.run(&[]));
    assert!(value["ok"].as_bool().unwrap(), "{value}");
    assert!(PathBuf::from(value["data"]["export_path"].as_str().unwrap()).starts_with(&generated));
    let value = fixture.parse(&fixture.run_edit(&[]));
    assert!(value["ok"].as_bool().unwrap(), "{value}");
    assert!(PathBuf::from(value["data"]["export_path"].as_str().unwrap()).starts_with(&edited));
    let value = fixture.parse(&fixture.run(&["--out", "explicit.png"]));
    assert!(value["ok"].as_bool().unwrap(), "{value}");
    assert_eq!(
        PathBuf::from(value["data"]["export_path"].as_str().unwrap()),
        fixture.root.join("explicit.png")
    );
}

#[test]
fn default_exports_use_the_invocation_directory_and_timestamp_names() {
    let fixture = Fixture::new("success");
    let first = fixture.parse(&fixture.run(&[]));
    let first_path = PathBuf::from(first["data"]["export_path"].as_str().unwrap());
    assert_eq!(first_path.parent().unwrap(), fixture.root);
    let name = first_path.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("image_"));
    assert!(name.ends_with(".png"));
    assert_eq!(name.len(), 19); // image_MMDD_HHMM.png
    let original = std::fs::read(&first_path).unwrap();
    let second = fixture.parse(&fixture.run(&[]));
    let second_path = PathBuf::from(second["data"]["export_path"].as_str().unwrap());
    assert_ne!(first_path, second_path);
    assert!(second_path.exists());
    assert_eq!(std::fs::read(first_path).unwrap(), original);
    let job = Store::open(&fixture.paths.database())
        .unwrap()
        .job_info(second["data"]["job_id"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(job["result"]["export_path"], second["data"]["export_path"]);
}

#[test]
fn image_generation_defaults_to_installed_model_and_records_asset() {
    let fixture = Fixture::new("success");
    let export = fixture.root.join("export.png");
    let out = fixture.run(&["--out", export.to_str().unwrap(), "--seed", "42"]);
    let value = fixture.parse(&out);
    assert!(out.status.success(), "{value}");
    assert_eq!(value["data"]["model"], gendock::image::DEFAULT_MODEL);
    assert_eq!(value["data"]["seed"], 42);
    assert_eq!(value["data"]["width"], 256);
    assert!(export.exists());
    let store = Store::open(&fixture.paths.database()).unwrap();
    let job = store
        .job_info(value["data"]["job_id"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(job["status"], "succeeded");
    assert_eq!(job["request"]["execution_mode"], "synchronous");
    assert!(job["request"]["worker_pid"].is_number());
    assert_eq!(job["result"]["asset"]["id"], value["data"]["asset"]["id"]);
    let conn = rusqlite::Connection::open(fixture.paths.database()).unwrap();
    let count: i64 = conn
        .query_row("SELECT count(*) FROM assets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        !String::from_utf8(out.stdout)
            .unwrap()
            .contains("worker-progress")
    );
}

#[test]
fn worker_failures_and_invalid_outputs_are_persisted() {
    for (mode, code) in [
        ("failure", "generation_failed"),
        ("invalid", "invalid_image_output"),
    ] {
        let fixture = Fixture::new(mode);
        let out = fixture.run(&[]);
        assert_eq!(out.status.code(), Some(1));
        let value = fixture.parse(&out);
        assert_eq!(value["error"]["code"], code);
        let job = Store::open(&fixture.paths.database())
            .unwrap()
            .job_info(value["error"]["details"]["job_id"].as_i64().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(job["status"], "failed");
    }
}

#[test]
fn preflight_preserves_existing_exports_and_reports_missing_runtime() {
    let fixture = Fixture::new("success");
    let output = fixture.root.join("keep.png");
    std::fs::write(&output, b"original").unwrap();
    let value = fixture.parse(&fixture.run(&["--out", output.to_str().unwrap()]));
    assert_eq!(value["error"]["code"], "output_exists");
    assert_eq!(std::fs::read(output).unwrap(), b"original");
    std::fs::remove_file(fixture.paths.data.join("runtimes/image.json")).unwrap();
    let value = fixture.parse(&fixture.run(&[]));
    assert_eq!(value["error"]["code"], "runtime_not_ready");
    assert!(
        Store::open(&fixture.paths.database())
            .unwrap()
            .jobs()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn export_races_preserve_the_generated_asset_and_existing_file() {
    let fixture = Fixture::new("export-conflict");
    let export = fixture.root.join("export.png");
    let out = fixture.run(&["--out", export.to_str().unwrap()]);
    let value = fixture.parse(&out);
    assert_eq!(value["error"]["code"], "export_failed");
    assert_eq!(std::fs::read(export).unwrap(), b"racer");
    let job = Store::open(&fixture.paths.database())
        .unwrap()
        .job_info(value["error"]["details"]["job_id"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(job["status"], "succeeded");
    assert!(std::path::Path::new(job["result"]["asset"]["path"].as_str().unwrap()).is_file());
}

#[test]
fn concurrent_generation_and_invalid_dimensions_do_not_start_workers() {
    let fixture = Fixture::new("success");
    std::fs::create_dir_all(&fixture.paths.state).unwrap();
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(fixture.paths.state.join("image-generation.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    assert_eq!(
        fixture.parse(&fixture.run(&[]))["error"]["code"],
        "generation_busy"
    );
    let out = fixture
        .command()
        .args([
            "image", "generate", "--prompt", "sea", "--width", "257", "--height", "256",
        ])
        .output()
        .unwrap();
    assert_eq!(
        fixture.parse(&out)["error"]["code"],
        "invalid_image_options"
    );
    assert!(!fixture.root.join("prompt.txt").exists());
    assert!(
        Store::open(&fixture.paths.database())
            .unwrap()
            .jobs()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn discovery_is_quick_but_generation_checks_full_weight_identity() {
    let fixture = Fixture::new("success");
    let store = Store::open(&fixture.paths.database()).unwrap();
    let directory = PathBuf::from(&store.models().unwrap()[0].path);
    std::fs::write(directory.join("weights.bin"), b"bad").unwrap();
    let out = fixture
        .command()
        .args(["model", "info", "--model", gendock::image::DEFAULT_MODEL])
        .output()
        .unwrap();
    let info = fixture.parse(&out);
    assert_eq!(info["data"]["execution_supported"], true);
    assert_eq!(info["data"]["compatibility"]["runtime_ready"], true);
    assert_eq!(info["data"]["files_available"], true);
    let failed = fixture.parse(&fixture.run(&[]));
    assert_eq!(failed["error"]["code"], "model_files_incomplete");
    assert!(store.jobs().unwrap().is_empty());
}

#[test]
fn prompt_is_an_argument_not_shell_code() {
    let fixture = Fixture::new("success");
    let prompt = "$(touch injected); ' quote \n next line";
    let out = fixture
        .command()
        .current_dir(&fixture.root)
        .args([
            "image", "generate", "--prompt", prompt, "--width", "256", "--height", "256",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("prompt.txt")).unwrap(),
        prompt
    );
    assert!(!fixture.root.join("injected").exists());
}

#[test]
fn sigterm_cancels_owned_worker_and_marks_job() {
    let fixture = Fixture::new("wait");
    let mut child = fixture
        .command()
        .args([
            "image", "generate", "--prompt", "sea", "--width", "256", "--height", "256",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !fixture.root.join("started").exists() {
        if child.try_wait().unwrap().is_some() {
            let out = child.wait_with_output().unwrap();
            panic!(
                "CLI exited before worker startup: {}",
                String::from_utf8_lossy(&out.stdout)
            );
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let out = child.wait_with_output().unwrap();
            panic!(
                "worker startup timed out: {}",
                String::from_utf8_lossy(&out.stdout)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        Command::new("/bin/kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let out = child.wait_with_output().unwrap();
    let value = fixture.parse(&out);
    assert_eq!(value["error"]["code"], "generation_cancelled");
    let store = Store::open(&fixture.paths.database()).unwrap();
    assert_eq!(store.jobs().unwrap()[0].status, "cancelled");
}

#[test]
fn stale_sync_jobs_recover_but_live_workers_are_not_killed() {
    let fixture = Fixture::new("success");
    let store = Store::open(&fixture.paths.database()).unwrap();
    let stale = store
        .start_image_job(&json!({"execution_mode":"synchronous"}))
        .unwrap();
    gendock::image::recover_interrupted(&store).unwrap();
    assert_eq!(
        store.job_info(stale).unwrap().unwrap()["status"],
        "interrupted"
    );
    let edit_stale = store
        .start_image_job_for("image.edit", &json!({"execution_mode":"synchronous"}))
        .unwrap();
    gendock::image::recover_interrupted(&store).unwrap();
    assert_eq!(
        store.job_info(edit_stale).unwrap().unwrap()["status"],
        "interrupted"
    );
    let live = store
        .start_image_job(&json!({"execution_mode":"synchronous","worker_pid":std::process::id()}))
        .unwrap();
    assert!(gendock::image::recover_interrupted(&store).is_err());
    assert_eq!(store.job_info(live).unwrap().unwrap()["status"], "running");
}
