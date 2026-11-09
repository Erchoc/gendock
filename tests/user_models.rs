use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    thread,
    time::Duration,
};

fn run(root: &Path, args: &[&str]) -> (i32, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gendock"));
    command
        .args(args)
        .env("HOME", root)
        .env("HF_TOKEN", "test-token-must-not-reach-custom-host");
    for kind in ["CONFIG", "DATA", "STATE", "CACHE"] {
        command.env(format!("XDG_{kind}_HOME"), root.join(kind));
    }
    let out = command.output().unwrap();
    assert!(out.stderr.is_empty());
    (
        out.status.code().unwrap(),
        serde_json::from_slice(&out.stdout).unwrap(),
    )
}

#[test]
fn user_manifest_is_searchable_downloads_direct_url_and_preserves_files_on_removal() {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/files/song.bin", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let start = std::time::Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(start.elapsed() < Duration::from_secs(5));
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 1);
            request.push(byte[0]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc")
            .unwrap();
        String::from_utf8(request).unwrap()
    });
    let manifest = root.path().join("song.json");
    let mut definition = json!({"id":"my-song","name":"My Song","type":"music","family":"song","homepage_url":"https://example.com/song","revision":"v1","backend":"custom","license":"unspecified",
        "files":[{"path":"song.bin","size":3,"hash":format!("{:x}",Sha256::digest(b"abc")),"algorithm":"sha256","url":url}]});
    std::fs::write(&manifest, serde_json::to_vec(&definition).unwrap()).unwrap();
    let manifest_path = manifest.to_str().unwrap();
    assert_eq!(
        run(root.path(), &["model", "add", "--manifest", manifest_path]).0,
        0
    );
    let (_, found) = run(root.path(), &["model", "search", "song"]);
    assert_eq!(found["data"]["models"][0]["type"], "music");
    assert_eq!(found["data"]["models"][0]["origin"], "user");
    let (status, download) = run(root.path(), &["model", "download", "--model", "my-song"]);
    assert_eq!(status, 0, "{download}");
    let request = server.join().unwrap().to_lowercase();
    assert!(request.starts_with("get /files/song.bin "));
    assert!(!request.contains("authorization:"));
    let installed = std::path::PathBuf::from(download["data"]["path"].as_str().unwrap());
    assert_eq!(std::fs::read(installed.join("song.bin")).unwrap(), b"abc");
    definition["description"] = json!("Updated description");
    definition["status"] = json!("deprecated");
    definition["replaced_by"] = json!("next-song");
    std::fs::write(&manifest, serde_json::to_vec(&definition).unwrap()).unwrap();
    assert_eq!(
        run(
            root.path(),
            &["model", "update", "--manifest", manifest_path]
        )
        .0,
        0
    );
    let (_, info) = run(root.path(), &["model", "info", "--model", "my-song"]);
    assert_eq!(info["data"]["status"], "deprecated");
    assert_eq!(info["data"]["replaced_by"], "next-song");
    assert_eq!(
        run(root.path(), &["model", "remove", "--model", "my-song"]).0,
        0
    );
    assert!(installed.join("song.bin").exists());
    assert_eq!(
        run(root.path(), &["model", "info", "--model", "my-song"]).1["error"]["code"],
        "model_not_found"
    );
}

#[test]
fn known_model_reference_does_not_require_backend_selection() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("weights");
    std::fs::create_dir(&directory).unwrap();
    let (status, result) = run(
        root.path(),
        &[
            "model",
            "add",
            "--name",
            "flux2-klein-4b-q4",
            "--path",
            directory.to_str().unwrap(),
        ],
    );
    assert_eq!(status, 0, "{result}");
    let (_, models) = run(root.path(), &["model", "list"]);
    assert_eq!(models["data"][0]["backend"], "mflux");
    let (_, info) = run(
        root.path(),
        &["model", "info", "--model", "flux2-klein-4b-q4"],
    );
    assert_eq!(info["data"]["origin"], "builtin");
}

#[test]
fn local_reference_metadata_and_path_can_be_maintained() {
    let root = tempfile::tempdir().unwrap();
    let one = root.path().join("one");
    let two = root.path().join("two");
    std::fs::write(&one, b"one").unwrap();
    std::fs::write(&two, b"two").unwrap();
    assert_eq!(
        run(
            root.path(),
            &[
                "model",
                "add",
                "--name",
                "my-audio",
                "--path",
                one.to_str().unwrap(),
                "--backend",
                "custom",
                "--type",
                "audio",
                "--homepage",
                "https://example.com/audio"
            ]
        )
        .0,
        0
    );
    let (_, found) = run(root.path(), &["model", "search", "my-audio"]);
    assert_eq!(found["data"]["models"][0]["type"], "audio");
    assert_eq!(found["data"]["models"][0]["downloadable"], false);
    assert_eq!(
        run(
            root.path(),
            &[
                "model",
                "update",
                "--name",
                "my-audio",
                "--path",
                two.to_str().unwrap()
            ]
        )
        .0,
        0
    );
    let (_, info) = run(root.path(), &["model", "info", "--model", "my-audio"]);
    assert_eq!(
        info["data"]["installation"]["path"].as_str().unwrap(),
        two.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(std::fs::read(one).unwrap(), b"one");
    let (_, protected) = run(
        root.path(),
        &["model", "remove", "--model", "flux2-klein-4b-q4"],
    );
    assert_eq!(protected["error"]["code"], "builtin_model_protected");
}
