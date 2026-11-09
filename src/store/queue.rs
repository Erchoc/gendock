use super::{Job, Store};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

fn error(
    code: &'static str,
    message: &'static str,
    next_step: &'static str,
    details: Value,
) -> anyhow::Error {
    crate::models::ModelError {
        code,
        message,
        retryable: false,
        next_step,
        details,
    }
    .into()
}

impl Store {
    pub fn submit_service(
        &self,
        kind: &str,
        payload: &Value,
        owner: &str,
        limit: u32,
    ) -> Result<i64> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.0,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let pending:u32=tx.query_row("SELECT count(*) FROM jobs WHERE status IN ('queued','running') AND json_extract(request,'$.queue.owner')=?1",[owner],|r|r.get(0))?;
        if pending >= limit {
            return Err(error(
                "queue_full",
                "The client's pending-job limit has been reached.",
                "Wait for current work to finish or cancel a queued job.",
                json!({"limit":limit}),
            ));
        }
        let request = json!({"execution_mode":"queued","payload":payload,"queue":{"owner":owner,"cancel_requested":false}});
        tx.execute(
            "INSERT INTO jobs(kind,request,status) VALUES (?1,?2,'queued')",
            params![kind, request.to_string()],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }
    pub fn claim_service(&self, server_id: &str) -> Result<Option<Job>> {
        let result=self.0.query_row(
            "UPDATE jobs SET status='running',updated_at=unixepoch(),request=json_set(request,'$.owner_pid',?2,'$.queue.server_id',?1,'$.queue.heartbeat',unixepoch(),'$.queue.lease_until',unixepoch()+10) WHERE id=(SELECT id FROM jobs WHERE status='queued' AND json_extract(request,'$.execution_mode')='queued' ORDER BY id LIMIT 1) AND NOT EXISTS(SELECT 1 FROM jobs WHERE status='running' AND json_extract(request,'$.execution_mode')='queued') RETURNING id,kind,request,status",
            params![server_id,std::process::id()],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?;
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
    pub fn service_tick(&self, id: i64, server_id: &str) -> Result<bool> {
        let requested:Option<bool>=self.0.query_row("UPDATE jobs SET request=json_set(request,'$.queue.heartbeat',unixepoch(),'$.queue.lease_until',unixepoch()+10) WHERE id=?1 AND status='running' AND json_extract(request,'$.queue.server_id')=?2 RETURNING json_extract(request,'$.queue.cancel_requested')",params![id,server_id],|r|r.get(0)).optional()?;
        requested.ok_or_else(|| {
            error(
                "job_ownership_lost",
                "The execution lease no longer belongs to this service.",
                "Inspect the job before retrying; the owned worker has been stopped.",
                json!({"job_id":id}),
            )
        })
    }
    pub fn enrich_service_job(&self, id: i64, server_id: &str, request: &Value) -> Result<()> {
        let mut request = request.clone();
        request["execution_mode"] = json!("queued");
        anyhow::ensure!(self.0.execute("UPDATE jobs SET request=json_patch(request,?3) WHERE id=?1 AND status='running' AND json_extract(request,'$.queue.server_id')=?2",params![id,server_id,request.to_string()])?==1,"queued job ownership changed");
        Ok(())
    }
    pub fn visible_job(&self, id: i64, owner: &str, admin: bool) -> Result<Option<Value>> {
        Ok(self
            .job_info(id)?
            .filter(|job| admin || job["request"]["queue"]["owner"] == owner))
    }
    pub fn visible_jobs(&self, owner: &str, admin: bool) -> Result<Vec<Value>> {
        let mut stmt=self.0.prepare("SELECT id FROM jobs WHERE ?2 OR json_extract(request,'$.queue.owner')=?1 ORDER BY id DESC LIMIT 100")?;
        let ids = stmt
            .query_map(params![owner, admin], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| self.job_info(id).map(|v| v.expect("selected job exists")))
            .collect()
    }
    pub fn cancel_service(&self, id: i64, owner: &str, admin: bool) -> Result<Option<Value>> {
        let Some(job) = self.visible_job(id, owner, admin)? else {
            return Ok(None);
        };
        if job["request"]["execution_mode"] != "queued" {
            return Err(error(
                "not_queued_job",
                "This job is owned by a direct synchronous CLI.",
                "Cancel its owning CLI with Ctrl+C.",
                json!({"job_id":id}),
            ));
        }
        self.0.execute("UPDATE jobs SET status='cancelled',error=?2,updated_at=unixepoch(),request=json_set(request,'$.queue.cancel_requested',json('true')) WHERE id=?1 AND status='queued'",params![id,json!({"code":"generation_cancelled","message":"Cancelled before execution."}).to_string()])?;
        self.0.execute("UPDATE jobs SET request=json_set(request,'$.queue.cancel_requested',json('true')),updated_at=unixepoch() WHERE id=?1 AND status='running'",[id])?;
        self.visible_job(id, owner, admin)
    }
    pub fn requeue_service(&self, id: i64) -> Result<()> {
        self.0.execute("UPDATE jobs SET status=CASE WHEN coalesce(json_extract(request,'$.queue.cancel_requested'),0) THEN 'cancelled' ELSE 'queued' END,error=CASE WHEN coalesce(json_extract(request,'$.queue.cancel_requested'),0) THEN ?2 ELSE error END,request=json_remove(request,'$.worker_pid','$.owner_pid','$.queue.server_id','$.queue.heartbeat','$.queue.lease_until'),updated_at=unixepoch() WHERE id=?1 AND status='running'",params![id,json!({"code":"generation_cancelled","message":"Cancelled before execution."}).to_string()])?;
        Ok(())
    }
    pub fn asset_file(&self, id: i64) -> Result<Option<String>> {
        Ok(self
            .0
            .query_row(
                "SELECT path FROM assets WHERE job_id=?1 ORDER BY created_at DESC LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .optional()?)
    }
}
