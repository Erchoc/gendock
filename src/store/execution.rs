use super::Store;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

impl Store {
    /// Synchronous executions are owned by the calling CLI, not the queue worker.
    pub fn start_image_job(&self, request: &Value) -> Result<i64> {
        self.start_image_job_for("image.generate", request)
    }
    pub fn start_image_job_for(&self, kind: &str, request: &Value) -> Result<i64> {
        anyhow::ensure!(
            ["image.generate", "image.edit"].contains(&kind),
            "unsupported synchronous image operation"
        );
        self.0.execute(
            "INSERT INTO jobs(kind,request,status) VALUES (?1,?2,'running')",
            params![kind, request.to_string()],
        )?;
        Ok(self.0.last_insert_rowid())
    }
    pub fn record_worker(&self, id: i64, pid: u32) -> Result<()> {
        anyhow::ensure!(self.0.execute("UPDATE jobs SET request=json_set(request,'$.worker_pid',?2),updated_at=unixepoch() WHERE id=?1 AND status='running'",params![id,pid])?==1,"job is not running");
        Ok(())
    }
    pub fn running_sync_jobs(&self) -> Result<Vec<(i64, Value)>> {
        let mut stmt=self.0.prepare("SELECT id,request FROM jobs WHERE status='running' AND kind IN ('image.generate','image.edit') AND json_extract(request,'$.execution_mode') IN ('synchronous','queued')")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.map(|row| {
            let (id, request) = row?;
            Ok((id, serde_json::from_str(&request)?))
        })
        .collect()
    }
    pub fn fail_image_job(&self, id: i64, status: &str, error: &Value) -> Result<()> {
        anyhow::ensure!(
            ["failed", "cancelled", "interrupted"].contains(&status),
            "invalid failure status"
        );
        self.0.execute("UPDATE jobs SET status=?2,error=?3,updated_at=unixepoch() WHERE id=?1 AND status='running'",params![id,status,error.to_string()])?;
        Ok(())
    }
    pub fn complete_image_job(
        &self,
        id: i64,
        asset_id: &str,
        path: &str,
        sha256: &str,
        result: &Value,
    ) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.0,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        anyhow::ensure!(tx.execute("UPDATE jobs SET status='succeeded',result=?2,updated_at=unixepoch() WHERE id=?1 AND status='running'",params![id,result.to_string()])?==1,"job is not running");
        tx.execute(
            "INSERT INTO assets(id,job_id,path,sha256,media_type) VALUES (?1,?2,?3,?4,'image/png')",
            params![asset_id, id, path, sha256],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn update_job_result(&self, id: i64, result: &Value) -> Result<()> {
        self.0.execute(
            "UPDATE jobs SET result=?2 WHERE id=?1 AND status='succeeded'",
            params![id, result.to_string()],
        )?;
        Ok(())
    }
    pub fn job_info(&self, id: i64) -> Result<Option<Value>> {
        let row = self
            .0
            .query_row(
                "SELECT kind,request,status,result,error FROM jobs WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(kind,request,status,result,error)|->Result<Value>{Ok(json!({"id":id,"kind":kind,"request":serde_json::from_str::<Value>(&request)?,"status":status,"result":result.map(|s|serde_json::from_str::<Value>(&s)).transpose()?,"error":error.map(|s|serde_json::from_str::<Value>(&s)).transpose()?}))}).transpose()
    }
}
