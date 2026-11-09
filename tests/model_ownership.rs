use gendock::{
    models::{ModelError, catalog, download::install, hub::Hub},
    paths::Paths,
    store::Store,
};
use std::fs::OpenOptions;

#[test]
fn concurrent_download_is_rejected_without_network() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root, |_| None);
    let model = catalog().remove(0);
    let directory = paths.data.join("models").join(&model.id);
    std::fs::create_dir_all(&directory).unwrap();
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join("download.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let error = install(&model, &Hub::new("hf", None).unwrap(), &paths, None).unwrap_err();
    assert_eq!(
        error.downcast_ref::<ModelError>().unwrap().code,
        "download_busy"
    );
    assert!(!paths.database().exists());
}

#[test]
fn external_registration_cannot_be_replaced_by_download() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root, |_| None);
    let model = catalog().remove(0);
    let shared = root.join("shared");
    std::fs::create_dir(&shared).unwrap();
    Store::open(&paths.database())
        .unwrap()
        .add_model(&model.id, &shared, "mflux")
        .unwrap();
    let error = install(&model, &Hub::new("hf", None).unwrap(), &paths, None).unwrap_err();
    assert_eq!(
        error.downcast_ref::<ModelError>().unwrap().code,
        "resource_conflict"
    );
    assert_eq!(
        Store::read_models(&paths.database()).unwrap()[0].ownership,
        "reference"
    );
    assert!(!paths.data.join("models").exists());
}

#[cfg(unix)]
#[test]
fn managed_symlink_does_not_overwrite_external_weights() {
    use gendock::models::download::fetch_file;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let model = catalog().remove(0);
    let file = &model.files[0];
    let shared = root.join("shared");
    std::fs::write(&shared, b"original").unwrap();
    std::os::unix::fs::symlink(&shared, root.join(&file.path)).unwrap();
    assert!(fetch_file(&Hub::new("hf", None).unwrap(), &model, file, &root).is_err());
    assert_eq!(std::fs::read(&shared).unwrap(), b"original");
}
