use super::Store;
use crate::models::{Model, ModelError, manifest};
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde_json::json;
use std::path::Path;

fn conflict(id: &str) -> anyhow::Error {
    ModelError {code:"builtin_model_protected",message:"User edits cannot replace or delete a built-in model definition.",retryable:false,
        next_step:"Import the customized definition under a different model ID. Remove only its local registration with model remove --name when needed.",details:json!({"model":id})}.into()
}

fn insert(conn: &rusqlite::Connection, model: &Model, update: bool) -> Result<()> {
    let sql = if update {
        "INSERT INTO model_catalog(id,origin,name,family,model_type,description,repository,status,recommended,definition) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET name=excluded.name,family=excluded.family,model_type=excluded.model_type,description=excluded.description,repository=excluded.repository,status=excluded.status,recommended=excluded.recommended,definition=excluded.definition,updated_at=unixepoch() WHERE model_catalog.origin=excluded.origin"
    } else {
        "INSERT INTO model_catalog(id,origin,name,family,model_type,description,repository,status,recommended,definition) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"
    };
    conn.execute(
        sql,
        params![
            model.id,
            model.origin,
            model.name,
            model.family,
            model.model_type,
            model.description,
            model.repository,
            model.status,
            model.recommended,
            serde_json::to_string(model)?
        ],
    )?;
    Ok(())
}

fn local_model(
    name: &str,
    backend: &str,
    model_type: &str,
    homepage: Option<&str>,
) -> Result<Model> {
    Ok(serde_json::from_value(
        json!({"id":name,"name":name,"origin":"user","family":name,"type":model_type,
        "homepage_url":homepage,"revision":"local","license":"unspecified","backend":backend}),
    )?)
}

impl Store {
    /// Monotonic release reconciliation. A stale binary never downgrades the catalog.
    pub fn sync_builtins(&self, revision: i64, models: &[Model]) -> Result<()> {
        for model in models {
            manifest::validate(model)?;
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.0,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let current: Option<i64> = tx
            .query_row(
                "SELECT builtin_revision FROM catalog_state WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if current.is_some_and(|current| current >= revision) {
            return Ok(());
        }
        tx.execute("UPDATE model_catalog SET status='deprecated',recommended=0,definition=json_set(definition,'$.status','deprecated','$.recommended',json('false'),'$.deprecation_reason','Removed from the current built-in catalog; local files are preserved.'),updated_at=unixepoch() WHERE origin='builtin' AND status='active'",[])?;
        for model in models {
            let mut model = model.clone();
            model.origin = "builtin".into();
            insert(&tx, &model, true)?;
        }
        tx.execute("INSERT INTO catalog_state(singleton,builtin_revision) VALUES (1,?1) ON CONFLICT(singleton) DO UPDATE SET builtin_revision=excluded.builtin_revision",[revision])?;
        tx.commit()?;
        Ok(())
    }

    pub fn catalog_search(&self, query: &str) -> Result<Vec<Model>> {
        let mut stmt=self.0.prepare("SELECT definition FROM model_catalog WHERE instr(lower(id || ' ' || name || ' ' || family || ' ' || description || ' ' || repository),lower(?1))>0 ORDER BY CASE status WHEN 'active' THEN 0 WHEN 'deprecated' THEN 1 ELSE 2 END,recommended DESC,id")?;
        let rows = stmt.query_map([query.trim()], |r| r.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn catalog_find(&self, id: &str) -> Result<Model> {
        // IDs are unique; repository aliases may become ambiguous across variants.
        let exact: Option<String> = self
            .0
            .query_row(
                "SELECT definition FROM model_catalog WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(value) = exact {
            return Ok(serde_json::from_str(&value)?);
        }
        let mut stmt = self
            .0
            .prepare("SELECT definition FROM model_catalog WHERE repository=?1 ORDER BY id")?;
        let matches = stmt
            .query_map([id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        match matches.as_slice() {
            [definition]=>Ok(serde_json::from_str(definition)?),
            []=>Err(ModelError {code:"model_not_found",message:"Model is not in the local catalog.",retryable:false,next_step:"Run gendock model search <keyword> or import a model manifest with model add --manifest.",details:json!({"model":id})}.into()),
            _=>Err(ModelError {code:"ambiguous_model",message:"Several model variants use this repository.",retryable:false,next_step:"Use an exact model ID from model search rather than the repository name.",details:json!({"repository":id})}.into()),
        }
    }

    pub fn import_model(&self, model: &Model, update: bool) -> Result<()> {
        let mut model = model.clone();
        model.origin = "user".into();
        manifest::validate(&model)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.0,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let origin: Option<String> = tx
            .query_row(
                "SELECT origin FROM model_catalog WHERE id=?1",
                [&model.id],
                |r| r.get(0),
            )
            .optional()?;
        if origin.as_deref() == Some("builtin") {
            return Err(conflict(&model.id));
        }
        if update && origin.is_none() {
            return Err(ModelError {
                code: "model_not_found",
                message: "Cannot update a model that is not registered.",
                retryable: false,
                next_step: "Use model add --manifest to create the definition first.",
                details: json!({"model":model.id}),
            }
            .into());
        }
        insert(&tx, &model, update)?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_catalog_model(&self, id: &str) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.0,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let origin: Option<String> = tx
            .query_row("SELECT origin FROM model_catalog WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        match origin.as_deref() {
            Some("builtin") => return Err(conflict(id)),
            None => anyhow::bail!("model not found: {id}"),
            _ => {}
        }
        tx.execute("DELETE FROM model_catalog WHERE id=?1", [id])?;
        tx.execute("DELETE FROM models WHERE name=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn add_local_model(
        &self,
        name: &str,
        path: &Path,
        backend: &str,
        model_type: &str,
        homepage: Option<&str>,
    ) -> Result<()> {
        let model = local_model(name, backend, model_type, homepage)?;
        manifest::validate(&model)?;
        let path = path.canonicalize().context("model path must exist")?;
        let path = path.to_str().context("model path must be valid UTF-8")?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.0,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        tx.execute(
            "INSERT INTO models(name,path,backend,ownership) VALUES (?1,?2,?3,'reference')",
            params![name, path, backend],
        )?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM model_catalog WHERE id=?1)",
            [name],
            |r| r.get(0),
        )?;
        if !exists {
            insert(&tx, &model, false)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn update_reference(&self, name: &str, path: &Path) -> Result<()> {
        let path = path.canonicalize().context("model path must exist")?;
        let count = self.0.execute(
            "UPDATE models SET path=?2 WHERE name=?1 AND ownership='reference'",
            params![
                name,
                path.to_str().context("model path must be valid UTF-8")?
            ],
        )?;
        anyhow::ensure!(
            count == 1,
            "reference not found or model is managed: {name}"
        );
        Ok(())
    }

    pub(super) fn backfill_local_catalog(&self) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.0,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        for registered in self.models()? {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM model_catalog WHERE id=?1)",
                [&registered.name],
                |r| r.get(0),
            )?;
            if !exists {
                let model = local_model(&registered.name, &registered.backend, "other", None)?;
                // Legacy registration names may contain characters not accepted by new IDs.
                insert(&tx, &model, false)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
