use gendock::store::Store;
use std::sync::{Arc, Barrier};

#[test]
fn simultaneous_workers_claim_each_job_once() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("queue.sqlite3");
    let store = Store::open(&path).unwrap();
    for _ in 0..20 {
        store
            .enqueue("image.generate", &serde_json::json!({"prompt":"sea"}))
            .unwrap();
    }
    let barrier = Arc::new(Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                barrier.wait();
                let mut ids = Vec::new();
                while let Some(job) = store.claim().unwrap() {
                    ids.push(job.id);
                }
                ids
            })
        })
        .collect();
    let mut ids: Vec<_> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    assert_eq!(ids.len(), 20);
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 20);
}

#[test]
fn fresh_database_uses_the_complete_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fresh.sqlite3");
    Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, gendock::store::SCHEMA_VERSION);
    let mut statement=conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap();
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        tables,
        ["assets", "catalog_state", "jobs", "model_catalog", "models"]
    );
    conn.prepare("SELECT result,error FROM jobs").unwrap();
}

#[test]
fn incompatible_development_schema_is_not_migrated_or_reset() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("older.sqlite3");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE preserved(value TEXT); INSERT INTO preserved VALUES ('keep'); PRAGMA user_version=1;").unwrap();
    let error = match Store::open(&path) {
        Ok(_) => panic!("old schema was accepted"),
        Err(error) => error,
    };
    assert_eq!(
        error
            .downcast_ref::<gendock::models::ModelError>()
            .unwrap()
            .code,
        "database_schema_incompatible"
    );
    let value: String = conn
        .query_row("SELECT value FROM preserved", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "keep");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='jobs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn newer_schema_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("future.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 999).unwrap();
    assert!(Store::open(&path).is_err());
}
