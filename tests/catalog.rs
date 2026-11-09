use gendock::{
    models::{Model, ModelError, catalog},
    store::Store,
};
use serde_json::json;

fn user_model(id: &str) -> Model {
    serde_json::from_value(json!({"id":id,"name":"My Song Model","type":"music","family":"custom-music",
        "homepage_url":"https://example.com/my-song","revision":"v1","backend":"custom","license":"unspecified",
        "files":[{"path":"model.bin","size":3,"algorithm":"sha256","hash":"a".repeat(64),"url":"https://models.example.com/model.bin"}]})).unwrap()
}

#[test]
fn upgrades_reconcile_builtins_without_overwriting_users_or_files() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("db.sqlite3");
    let store = Store::open(&db).unwrap();
    let custom = user_model("my-song");
    store.import_model(&custom, false).unwrap();
    let local = root.path().join("shared.bin");
    std::fs::write(&local, b"shared").unwrap();
    let initial = catalog();
    store.add_model(&initial[1].id, &local, "mflux").unwrap();
    let mut preferred = initial[0].clone();
    preferred.recommended = true;
    preferred.description = "New recommendation".into();
    let mut new = initial[0].clone();
    new.id = "new-flux".into();
    store
        .sync_builtins(
            gendock::models::BUILTIN_CATALOG_REVISION + 1,
            &[preferred.clone(), new.clone()],
        )
        .unwrap();
    assert!(store.catalog_find(&preferred.id).unwrap().recommended);
    let previous = store.catalog_find(&initial[1].id).unwrap();
    assert_eq!(previous.status, "deprecated");
    assert!(!previous.recommended);
    assert!(previous.deprecation_reason.is_some());
    assert_eq!(store.catalog_find("my-song").unwrap().origin, "user");
    assert_eq!(store.catalog_find("new-flux").unwrap().origin, "builtin");
    assert_eq!(store.models().unwrap()[0].ownership, "reference");
    assert_eq!(std::fs::read(&local).unwrap(), b"shared");
    // An older binary's catalog must not resurrect the deprecated entry.
    store
        .sync_builtins(gendock::models::BUILTIN_CATALOG_REVISION, &initial)
        .unwrap();
    drop(store);
    let reopened = Store::open(&db).unwrap();
    assert_eq!(
        reopened.catalog_find(&initial[1].id).unwrap().status,
        "deprecated"
    );
    assert_eq!(
        reopened.catalog_find(&preferred.id).unwrap().description,
        "New recommendation"
    );
}

#[test]
fn user_definitions_are_searchable_editable_and_protected_from_collision() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("db.sqlite3")).unwrap();
    let mut custom = user_model("my-song");
    custom.origin = "builtin".into();
    store.import_model(&custom, false).unwrap();
    assert_eq!(store.catalog_find("my-song").unwrap().origin, "user");
    assert_eq!(store.catalog_search("song").unwrap()[0].model_type, "music");
    assert!(store.import_model(&custom, false).is_err());
    custom.name = "Updated Song Model".into();
    custom.status = "deprecated".into();
    custom.replaced_by = Some("new-song".into());
    store.import_model(&custom, true).unwrap();
    assert_eq!(
        store.catalog_find("my-song").unwrap().name,
        "Updated Song Model"
    );
    assert_eq!(
        store
            .catalog_find("my-song")
            .unwrap()
            .replaced_by
            .as_deref(),
        Some("new-song")
    );
    let builtin = catalog().remove(0);
    let error = store.import_model(&builtin, true).unwrap_err();
    assert_eq!(
        error.downcast_ref::<ModelError>().unwrap().code,
        "builtin_model_protected"
    );
    assert!(store.remove_catalog_model(&builtin.id).is_err());
    let local = root.path().join("my-song.bin");
    std::fs::write(&local, b"original").unwrap();
    store.add_model("my-song", &local, "custom").unwrap();
    store.remove_catalog_model("my-song").unwrap();
    assert!(store.catalog_find("my-song").is_err());
    assert!(store.models().unwrap().is_empty());
    assert_eq!(std::fs::read(local).unwrap(), b"original");
}

#[test]
fn new_builtin_collision_preserves_user_definition() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("db.sqlite3")).unwrap();
    store
        .import_model(&user_model("future-builtin"), false)
        .unwrap();
    let mut models = catalog();
    let mut added = models[0].clone();
    added.id = "future-builtin".into();
    models.push(added);
    store
        .sync_builtins(gendock::models::BUILTIN_CATALOG_REVISION + 1, &models)
        .unwrap();
    assert_eq!(store.catalog_find("future-builtin").unwrap().origin, "user");
    assert_eq!(
        store.catalog_find("future-builtin").unwrap().model_type,
        "music"
    );
}

#[test]
fn reopens_current_schema_preserving_queue_and_external_registration() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("legacy.sqlite3");
    let shared = root.path().join("legacy.bin");
    std::fs::write(&shared, b"legacy").unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(include_str!("../migrations/001_initial.sql"))
        .unwrap();
    conn.execute(
        "INSERT INTO models(name,path,backend,ownership) VALUES ('legacy',?1,'mflux','reference')",
        [shared.to_str().unwrap()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO jobs(kind,request,status) VALUES ('image.generate','{}','queued')",
        [],
    )
    .unwrap();
    drop(conn);
    let store = Store::open(&db).unwrap();
    assert_eq!(store.catalog_find("legacy").unwrap().origin, "user");
    assert_eq!(store.models().unwrap()[0].path, shared.to_str().unwrap());
    assert_eq!(store.claim().unwrap().unwrap().status, "running");
    assert_eq!(std::fs::read(shared).unwrap(), b"legacy");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, gendock::store::SCHEMA_VERSION);
}

#[test]
fn retired_and_local_only_models_do_not_start_downloads() {
    use gendock::{
        models::{download::install, hub::Hub},
        paths::Paths,
    };
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(root.path(), |_| None);
    let hub = Hub::new("hf", None).unwrap();
    let mut model = user_model("retired-song");
    model.status = "retired".into();
    let error = install(&model, &hub, &paths, None).unwrap_err();
    assert_eq!(
        error.downcast_ref::<ModelError>().unwrap().code,
        "model_retired"
    );
    model.status = "active".into();
    model.files.clear();
    let error = install(&model, &hub, &paths, None).unwrap_err();
    assert_eq!(
        error.downcast_ref::<ModelError>().unwrap().code,
        "model_not_downloadable"
    );
    assert!(!paths.data.exists());
}

#[test]
fn invalid_manifests_are_rejected_before_insertion() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(&root.path().join("db.sqlite3")).unwrap();
    for field in ["id", "type", "status", "url", "hash", "path"] {
        let mut model = user_model("invalid");
        match field {
            "id" => model.id = "../escape".into(),
            "type" => model.model_type = "random".into(),
            "status" => {
                model.status = "deprecated".into();
                model.recommended = true;
            }
            "url" => model.files[0].url = Some("file:///etc/passwd".into()),
            "hash" => model.files[0].hash = "bad".into(),
            "path" => model.files[0].path = "../escape".into(),
            _ => unreachable!(),
        }
        assert!(
            store.import_model(&model, false).is_err(),
            "accepted invalid {field}"
        );
    }
    assert!(store.catalog_search("invalid").unwrap().is_empty());
}
