mod catalog;
mod execution;
mod queue;

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

/// Pre-1.0 schema snapshot marker, independent of the application version.
pub const SCHEMA_VERSION: i64 = 3;

pub struct Store(Connection);

#[derive(Debug, Serialize)]
pub struct Model {
    pub name: String,
    pub path: String,
    pub backend: String,
    pub ownership: String,
}

#[derive(Debug, Serialize)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    pub request: serde_json::Value,
    pub status: String,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path).context("opening database")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 0 {
            tx.execute_batch(include_str!("../migrations/001_initial.sql"))?;
        } else if version != SCHEMA_VERSION {
            return Err(crate::models::ModelError {
                code:"database_schema_incompatible",
                message:"The local development database uses an incompatible schema.",
                retryable:false,
                next_step:"Back up the database first. Use a compatible development build or explicitly rebuild only the development database. Do not delete model or media files. Automatic migrations begin after 1.0.",
                details:serde_json::json!({"database_path":path,"found_schema":version,"expected_schema":SCHEMA_VERSION}),
            }.into());
        }
        tx.commit()?;
        let store = Self(conn);
        store.sync_builtins(
            crate::models::BUILTIN_CATALOG_REVISION,
            &crate::models::catalog(),
        )?;
        store.backfill_local_catalog()?;
        Ok(store)
    }

    /// Register a reference only. Never copy, alter or delete the target.
    pub fn add_model(&self, name: &str, path: &Path, backend: &str) -> Result<()> {
        self.add_local_model(name, path, backend, "other", None)
    }

    /// Register verified files owned by gendock; never replace an external reference.
    pub fn register_managed(&self, name: &str, path: &Path, backend: &str) -> Result<()> {
        let path = path.to_str().context("model path must be valid UTF-8")?;
        let count = self.0.execute(
            "INSERT INTO models(name,path,backend,ownership) VALUES (?1,?2,?3,'managed') ON CONFLICT(name) DO UPDATE SET path=excluded.path,backend=excluded.backend WHERE models.ownership='managed'",
            params![name,path,backend],
        )?;
        anyhow::ensure!(
            count == 1,
            "a referenced model already uses this name; remove or rename its registration first"
        );
        Ok(())
    }

    pub fn read_models(path: &Path) -> Result<Vec<Model>> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Self(conn).models()
    }

    pub fn models(&self) -> Result<Vec<Model>> {
        let mut stmt = self
            .0
            .prepare("SELECT name,path,backend,ownership FROM models ORDER BY name")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(Model {
                    name: r.get(0)?,
                    path: r.get(1)?,
                    backend: r.get(2)?,
                    ownership: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn remove_model(&self, name: &str) -> Result<()> {
        if self.0.execute("DELETE FROM models WHERE name=?1", [name])? == 0 {
            bail!("model not registered: {name}");
        }
        Ok(())
    }

    pub fn enqueue(&self, kind: &str, request: &serde_json::Value) -> Result<i64> {
        self.0.execute(
            "INSERT INTO jobs(kind,request,status) VALUES (?1,?2,'queued')",
            params![kind, request.to_string()],
        )?;
        Ok(self.0.last_insert_rowid())
    }

    /// The single UPDATE is atomic across independent database connections.
    pub fn claim(&self) -> Result<Option<Job>> {
        let result = self.0.query_row(
            "UPDATE jobs SET status='running',updated_at=unixepoch() WHERE id=(SELECT id FROM jobs WHERE status='queued' ORDER BY id LIMIT 1) AND status='queued' RETURNING id,kind,request,status",
            [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))).optional()?;
        result
            .map(|(id, kind, request, status)| {
                Ok(Job {
                    id,
                    kind,
                    request: serde_json::from_str(&request)?,
                    status,
                })
            })
            .transpose()
    }

    pub fn jobs(&self) -> Result<Vec<Job>> {
        let mut stmt = self
            .0
            .prepare("SELECT id,kind,request,status FROM jobs ORDER BY id DESC LIMIT 100")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, kind, request, status) = row?;
            Ok(Job {
                id,
                kind,
                request: serde_json::from_str(&request)?,
                status,
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_removal_preserves_original() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let model = dir.path().join("weights.bin");
        std::fs::write(&model, "original")?;
        let store = Store::open(&dir.path().join("db.sqlite3"))?;
        store.add_model("demo", &model, "mflux")?;
        store.remove_model("demo")?;
        assert_eq!(std::fs::read_to_string(model)?, "original");
        Ok(())
    }

    #[test]
    fn queue_persists_and_connections_cannot_claim_same_job() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let db = dir.path().join("db.sqlite3");
        let first = Store::open(&db)?;
        let id = first.enqueue("image.generate", &serde_json::json!({"prompt":"sea"}))?;
        drop(first);
        let a = Store::open(&db)?;
        let b = Store::open(&db)?;
        assert_eq!(a.claim()?.unwrap().id, id);
        assert!(b.claim()?.is_none());
        let mode: String = b.0.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        assert_eq!(mode, "wal");
        Ok(())
    }
}
