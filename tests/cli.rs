use serde_json::Value;
use std::process::{Command, Output};

fn run(args: &[&str], root: &std::path::Path) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gendock"));
    cmd.args(args)
        .env("HOME", root)
        .env_remove("GENDOCK_URL")
        .env_remove("GENDOCK_TOKEN");
    for kind in ["CONFIG", "DATA", "CACHE", "STATE"] {
        cmd.env(format!("XDG_{kind}_HOME"), root.join(kind));
    }
    cmd.output().unwrap()
}

fn parse(output: &Output) -> Value {
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {:?}",
        output.stderr
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be one JSON document");
    assert_eq!(value["schema_version"], 1);
    assert!(value.get("data").is_some());
    if value["ok"] == false {
        assert!(value["data"].is_null());
        let error = &value["error"];
        assert!(error["code"].is_string());
        assert!(!error["message"].as_str().unwrap().contains('\n'));
        assert!(error["retryable"].is_boolean());
        assert!(error["details"].is_object());
        assert!(!error["next_steps"].as_array().unwrap().is_empty());
    } else {
        assert!(value["error"].is_null());
    }
    value
}

#[test]
fn help_is_text_but_version_and_invalid_arguments_are_json() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        &[][..],
        &["--help"],
        &["model"],
        &["model", "--help"],
        &["help", "model"],
        &["model", "add", "--help"],
    ] {
        let result = run(args, root.path());
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        let text = std::str::from_utf8(&result.stdout).unwrap();
        assert!(text.contains("Usage:"));
        assert!(serde_json::from_str::<Value>(text).is_err());
    }
    let result = run(&["--version"], root.path());
    assert_eq!(parse(&result)["ok"], true);
    let result = run(&["image", "generate"], root.path());
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(parse(&result)["error"]["code"], "invalid_arguments");
    assert!(!root.path().join("DATA").exists());
}

#[test]
fn errors_offer_structured_recovery_without_rendered_usage() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        &["unknown"][..],
        &["server", "start", "--port", "oops"],
        &[
            "model",
            "add",
            "--name",
            "demo",
            "--path",
            "/missing-gendock-model",
            "--backend",
            "mflux",
        ],
    ] {
        let result = run(args, root.path());
        assert!(!result.status.success());
        let value = parse(&result);
        assert_eq!(value["ok"], false);
        assert!(
            !value["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Usage:")
        );
    }
    let missing = parse(&run(&["image", "generate"], root.path()));
    assert!(missing["error"]["details"]["arguments"].is_array());
}

#[test]
fn model_discovery_seeds_sqlite_and_reports_pinned_files() {
    let root = tempfile::tempdir().unwrap();
    let result = parse(&run(&["model", "search", "--query", "FLUX"], root.path()));
    let models = result["data"]["models"].as_array().unwrap();
    assert!(models.len() >= 2);
    let id = models[0]["id"].as_str().unwrap();
    let details = parse(&run(&["model", "info", "--model", id], root.path()));
    assert_eq!(details["data"]["revision"].as_str().unwrap().len(), 40);
    assert!(!details["data"]["files"].as_array().unwrap().is_empty());
    assert_eq!(details["data"]["inference_ready"], false);
    let missing = run(
        &["model", "download", "--model", "not-a-catalog-model"],
        root.path(),
    );
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(parse(&missing)["error"]["code"], "model_not_found");
    let positional = parse(&run(&["model", "search", "flux"], root.path()));
    assert_eq!(positional["data"]["source"], "sqlite_catalog");
    assert_eq!(
        positional["data"]["models"].as_array().unwrap().len(),
        models.len()
    );
    assert_eq!(positional["data"]["models"][0]["type"], "image");
    assert!(
        root.path()
            .join("DATA/gendock/database/gendock.sqlite3")
            .is_file()
    );
}

#[test]
fn pending_commands_have_no_storage_side_effects() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        &["model", "load", "--model", "demo"][..],
        &["text", "generate", "--prompt", "sea"],
        &["image", "upscale", "--input", "missing.png"],
    ] {
        let result = run(args, root.path());
        assert_eq!(result.status.code(), Some(3));
        assert_eq!(parse(&result)["error"]["code"], "not_implemented");
    }
    assert!(!root.path().join("DATA").exists());
}

#[test]
fn model_registration_roundtrip_preserves_shared_files() {
    let root = tempfile::tempdir().unwrap();
    let model = root.path().join("shared.bin");
    std::fs::write(&model, b"weights").unwrap();
    let args = [
        "model",
        "add",
        "--name",
        "demo",
        "--path",
        model.to_str().unwrap(),
        "--backend",
        "mflux",
    ];
    assert!(parse(&run(&args, root.path()))["ok"].as_bool().unwrap());
    let duplicate = run(&args, root.path());
    assert_eq!(duplicate.status.code(), Some(1));
    assert_eq!(parse(&duplicate)["ok"], false);
    assert_eq!(
        parse(&run(&["model", "list"], root.path()))["data"][0]["name"],
        "demo"
    );
    assert_eq!(
        parse(&run(&["model", "remove", "--name", "demo"], root.path()))["data"]["files_deleted"],
        false
    );
    assert_eq!(std::fs::read(&model).unwrap(), b"weights");
}
