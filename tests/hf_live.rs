//! Opt-in network smoke check: downloads only a small model README, never weights.
use gendock::models::{catalog, download::fetch_file, hub::Hub};

#[test]
#[ignore = "requires official Hugging Face network access; README only"]
fn official_pinned_readme_matches_catalog_hash() {
    let model = catalog().remove(0);
    let file = model
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .unwrap();
    assert!(file.size < 16384);
    let root = tempfile::tempdir().unwrap();
    fetch_file(
        &Hub::new("hf", None).unwrap(),
        &model,
        file,
        &root.path().canonicalize().unwrap(),
    )
    .unwrap();
}
