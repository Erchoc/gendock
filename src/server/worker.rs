use super::{App, Submission, auth, event, upload_path};
use crate::{image, store::Store};
use serde_json::json;
use std::{
    fs::OpenOptions,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

pub fn spawn(app: Arc<App>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !app.stop.load(Ordering::Relaxed) {
            if let Err(error) = step(&app) {
                event(
                    &app.paths,
                    "scheduler_error",
                    json!({"code":crate::cli::output::operation_error(&error)["error"]["code"]}),
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })
}
fn step(app: &Arc<App>) -> anyhow::Result<()> {
    // Inspect orphaned work only while no other gendock image caller owns the GPU lock.
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(app.paths.state.join("image-generation.lock"))?;
    if fs2::FileExt::try_lock_exclusive(&lock).is_err() {
        return Ok(());
    }
    let store = Store::open(&app.paths.database())?;
    if image::recover_interrupted(&store).is_err() {
        return Ok(());
    }
    let job = store.claim_service(&app.receipt.server_id)?;
    drop(lock);
    let Some(job) = job else { return Ok(()) };
    event(&app.paths, "job_started", json!({"job_id":job.id}));
    let revision = job.request["payload"]["revision"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let mut payload = job.request["payload"].clone();
    let outcome = (|| -> anyhow::Result<serde_json::Value> {
        let Some(fields) = payload.as_object_mut() else {
            anyhow::bail!("queued payload is not an object")
        };
        fields.remove("revision");
        let submission: Submission = serde_json::from_value(payload)?;
        let owner = auth::Principal {
            name: job.request["queue"]["owner"].as_str().unwrap_or("").into(),
            admin: false,
        };
        let input = submission
            .input_upload
            .as_deref()
            .map(|id| upload_path(&app.paths, &owner.name, id))
            .transpose()?;
        image::execute_queued(
            submission.options(),
            input,
            image::Queued {
                id: job.id,
                server_id: app.receipt.server_id.clone(),
                revision,
                shutdown: app.stop.clone(),
            },
        )
    })();
    if let Err(error) = outcome {
        let mut error = crate::cli::output::operation_error(&error)["error"].clone();
        if error["code"] == "generation_busy" && !app.stop.load(Ordering::Relaxed) {
            store.requeue_service(job.id)?;
            return Ok(());
        }
        let status = if app.stop.load(Ordering::Relaxed) && error["code"] != "generation_cancelled"
        {
            "interrupted"
        } else if error["code"] == "generation_cancelled" {
            "cancelled"
        } else {
            "failed"
        };
        error["details"]["job_id"] = json!(job.id);
        store.fail_image_job(job.id, status, &error)?;
    }
    if let Some(current) = store.job_info(job.id)? {
        event(
            &app.paths,
            "job_finished",
            json!({"job_id":job.id,"status":current["status"]}),
        );
    }
    Ok(())
}
