use gendock::{cli::ConfigCommand, config, paths::Paths};
use serde_json::{Value, json};
use std::{process::Command, time::Duration};

fn command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gendock"));
    command
        .env("HOME", root)
        .env_remove("GENDOCK_URL")
        .env_remove("GENDOCK_TOKEN");
    for kind in ["CONFIG", "DATA", "CACHE", "STATE"] {
        command.env(format!("XDG_{kind}_HOME"), root.join(kind));
    }
    command
}

#[test]
fn remote_endpoint_and_optional_key_are_used_without_local_storage() {
    for token in [None, Some("fixture-secret")] {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), |key| {
            key.strip_prefix("XDG_")
                .and_then(|s| s.strip_suffix("_HOME"))
                .map(|kind| temp.path().join(kind))
        });
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        config::execute(
            &paths,
            ConfigCommand::Set {
                key: "remote.endpoint".into(),
                value: endpoint,
            },
        )
        .unwrap();
        if let Some(token) = token {
            config::execute(
                &paths,
                ConfigCommand::Set {
                    key: "remote.api_key".into(),
                    value: token.into(),
                },
            )
            .unwrap();
        }
        let handler = std::thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .expect("client request");
            let authorization = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization"))
                .map(|h| h.value.as_str().to_owned());
            assert_eq!(authorization, token.map(|t| format!("Bearer {t}")));
            let response =
                gendock::cli::output::success(json!({"running": true, "server_id": "fixture"}));
            request
                .respond(tiny_http::Response::from_string(response.to_string()))
                .unwrap();
        });
        let output = command(temp.path())
            .args(["server", "status"])
            .output()
            .unwrap();
        handler.join().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["data"]["server_id"], "fixture");
        assert!(!paths.database().exists());
        let output = command(temp.path())
            .args(["config", "list"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture-secret"));
    }
}
