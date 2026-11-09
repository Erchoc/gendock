use gendock::{
    models::{
        File, Model, ModelError,
        download::{fetch_file, install, verify},
        hub::Hub,
    },
    paths::Paths,
    store::Store,
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn model(bytes: &[u8]) -> Model {
    Model {
        id: "fixture".into(),
        name: "Fixture".into(),
        homepage_url: Some("https://example.com/fixture".into()),
        repository: "test/fixture".into(),
        revision: "a".repeat(40),
        license: "apache-2.0".into(),
        backend: "mflux".into(),
        tasks: vec!["image.generate".into()],
        minimum_memory_gib: 1,
        files: vec![File {
            path: "weights.bin".into(),
            size: bytes.len() as u64,
            hash: format!("{:x}", Sha256::digest(bytes)),
            algorithm: "sha256".into(),
            url: None,
        }],
        ..Model::default()
    }
}

fn server(response: Vec<u8>) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let start = Instant::now();
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
            let mut b = [0];
            if stream.read(&mut b).unwrap() == 0 {
                break;
            }
            request.push(b[0]);
        }
        stream.write_all(&response).unwrap();
        String::from_utf8(request).unwrap()
    });
    (format!("http://{address}"), handle)
}
fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n").into_bytes();
    out.extend(body);
    out
}
fn paths(root: &Path) -> Paths {
    Paths::resolve(root, |_| None)
}

#[test]
fn install_verifies_activates_and_registers_managed_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let model = model(b"weights");
    let (url, handle) = server(response("200 OK", "Content-Length: 7\r\n", b"weights"));
    let hub = Hub::new(&url, None).unwrap();
    let paths = paths(&root);
    let result = install(&model, &hub, &paths, None).unwrap();
    assert_eq!(result["integrity_checked"], true);
    assert_eq!(result["inference_ready"], false);
    let request = handle.join().unwrap();
    assert!(request.contains(&format!(
        "/test/fixture/resolve/{}/weights.bin",
        model.revision
    )));
    assert!(!request.to_lowercase().contains("authorization:"));
    assert_eq!(
        Store::open(&paths.database()).unwrap().models().unwrap()[0].ownership,
        "managed"
    );
    // Re-running verifies existing files without needing the server.
    assert!(install(&model, &hub, &paths, None).is_ok());
}

#[test]
fn resumes_partial_download_and_checks_content_range() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let model = model(b"abcdef");
    std::fs::write(root.join("weights.bin.part"), b"abc").unwrap();
    let (url, handle) = server(response(
        "206 Partial Content",
        "Content-Length: 3\r\nContent-Range: bytes 3-5/6\r\n",
        b"def",
    ));
    fetch_file(
        &Hub::new(&url, None).unwrap(),
        &model,
        &model.files[0],
        &root,
    )
    .unwrap();
    assert!(
        handle
            .join()
            .unwrap()
            .to_lowercase()
            .contains("range: bytes=3-")
    );
    assert!(verify(&root.join("weights.bin"), &model.files[0]).unwrap());
}

#[test]
fn server_ignoring_range_restarts_safely() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let model = model(b"abcdef");
    std::fs::write(root.join("weights.bin.part"), b"abc").unwrap();
    let (url, handle) = server(response("200 OK", "Content-Length: 6\r\n", b"abcdef"));
    fetch_file(
        &Hub::new(&url, None).unwrap(),
        &model,
        &model.files[0],
        &root,
    )
    .unwrap();
    handle.join().unwrap();
    assert_eq!(std::fs::read(root.join("weights.bin")).unwrap(), b"abcdef");
}

#[test]
fn corruption_never_activates_or_registers_model() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let model = model(b"weights");
    let (url, handle) = server(response("200 OK", "Content-Length: 7\r\n", b"corrupt"));
    let paths = paths(&root);
    let error = install(&model, &Hub::new(&url, None).unwrap(), &paths, None).unwrap_err();
    assert_eq!(
        error.downcast_ref::<ModelError>().unwrap().code,
        "integrity_check_failed"
    );
    handle.join().unwrap();
    assert!(
        !paths
            .data
            .join("models/fixture")
            .join(&model.revision)
            .exists()
    );
    assert!(!paths.database().exists());
}

#[test]
fn interrupted_transfer_keeps_partial_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let model = model(b"abcdef");
    let (url, handle) = server(response("200 OK", "Content-Length: 6\r\n", b"abc"));
    assert!(
        fetch_file(
            &Hub::new(&url, None).unwrap(),
            &model,
            &model.files[0],
            &root
        )
        .is_err()
    );
    handle.join().unwrap();
    assert_eq!(
        std::fs::read(root.join("weights.bin.part")).unwrap(),
        b"abc"
    );
    assert!(!root.join("weights.bin").exists());
}

#[test]
fn invalid_range_and_access_denial_are_actionable() {
    for (status, headers, code) in [
        (
            "206 Partial Content",
            "Content-Length: 3\r\nContent-Range: bytes 0-2/6\r\n",
            "invalid_download_range",
        ),
        (
            "403 Forbidden",
            "Content-Length: 3\r\n",
            "model_access_denied",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(root.join("weights.bin.part"), b"abc").unwrap();
        let model = model(b"abcdef");
        let (url, handle) = server(response(status, headers, b"def"));
        let error = fetch_file(
            &Hub::new(&url, None).unwrap(),
            &model,
            &model.files[0],
            &root,
        )
        .unwrap_err();
        handle.join().unwrap();
        assert_eq!(error.downcast_ref::<ModelError>().unwrap().code, code);
        assert_eq!(
            std::fs::read(root.join("weights.bin.part")).unwrap(),
            b"abc"
        );
    }
}

#[test]
fn git_blob_checksums_cover_small_files() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().canonicalize().unwrap().join("config.json");
    std::fs::write(&file, b"{}").unwrap();
    let mut hash = sha1::Sha1::new();
    hash.update(b"blob 2\0{}");
    let manifest = File {
        path: "config.json".into(),
        size: 2,
        hash: format!("{:x}", hash.finalize()),
        algorithm: "git_sha1".into(),
        url: None,
    };
    assert!(verify(&file, &manifest).unwrap());
}
