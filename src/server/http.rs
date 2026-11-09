use super::{App, access::AccessRequest, auth, error, public_job};
use crate::{cli::output, store::Store};
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use tiny_http::{Header, Request, Response};

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}
fn json_response(request: AccessRequest, status: u16, value: Value) {
    let response = Response::from_string(value.to_string())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json"));
    let response = if status == 401 {
        response.with_header(header("WWW-Authenticate", "Bearer realm=\"gendockd\""))
    } else {
        response
    };
    let _ = request.respond(response);
}
pub fn reject(request: Request, error: anyhow::Error, paths: &crate::paths::Paths) {
    respond_error(AccessRequest::new(request, paths), error)
}
fn respond_error(mut request: AccessRequest, error: anyhow::Error) {
    let mut value = output::operation_error(&error);
    let code = value["error"]["code"].as_str().unwrap_or("");
    request.code = Some(code.to_owned());
    let status = match code {
        "unauthorized" => 401,
        "forbidden" => 403,
        "job_not_found" | "upload_not_found" | "route_not_found" => 404,
        "queue_full" => 429,
        "server_busy" | "storage_busy" => 503,
        "storage_error" | "filesystem_error" | "permission_denied" | "operation_failed"
        | "path_not_found" => 500,
        _ => 400,
    };
    let details = value["error"]["details"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter(|(key, _)| {
                    ["model", "job_id", "limit", "operation", "field"].contains(&key.as_str())
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<serde_json::Map<_, _>>()
        })
        .unwrap_or_default();
    if value["error"]["code"] == "model_not_installed" {
        value["error"]["next_steps"][0]["action"] = json!("prepare_shared_device_model");
        value["error"]["next_steps"][0]["description"] = json!(
            "Ask the shared-device administrator to run this command on that device, then resubmit."
        );
    }
    value["error"]["details"] = json!(details);
    json_response(request, status, value);
}
fn principal(request: &Request, app: &App) -> Result<auth::Principal> {
    let value = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .and_then(|h| h.value.as_str().strip_prefix("Bearer "));
    if let Some(value) = value
        && let Some(principal) = auth::authenticate(&app.paths, value)?
    {
        return Ok(principal);
    }
    Err(error(
        "unauthorized",
        "A valid access token is required.",
        "Set GENDOCK_TOKEN to the token issued by the device administrator.",
        json!({}),
    ))
}
fn body(request: &mut AccessRequest, limit: usize) -> Result<Vec<u8>> {
    if request.body_length().is_some_and(|length| length > limit) {
        return Err(error(
            "request_too_large",
            "The request exceeds its size limit.",
            "Submit a smaller payload.",
            json!({}),
        ));
    }
    let mut bytes = Vec::new();
    request
        .as_reader()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(error(
            "request_too_large",
            "The request exceeds its size limit.",
            "Submit a smaller payload.",
            json!({}),
        ));
    }
    Ok(bytes)
}
fn find_job(store: &Store, id: i64, principal: &auth::Principal) -> Result<Value> {
    store
        .visible_job(id, &principal.name, principal.admin)?
        .ok_or_else(|| {
            error(
                "job_not_found",
                "The job does not exist or is not accessible.",
                "Use job list with the same client identity.",
                json!({"job_id":id}),
            )
        })
}
pub fn handle(request: Request, app: Arc<App>) {
    let mut request = AccessRequest::new(request, &app.paths);
    let principal = match principal(&request, &app) {
        Ok(p) => p,
        Err(e) => {
            respond_error(request, e);
            return;
        }
    };
    request.principal = Some(principal.name.clone());
    let path = request.url().split('?').next().unwrap_or("").to_owned();
    let method = request.method().as_str().to_owned();
    let store = match Store::open(&app.paths.database()) {
        Ok(s) => s,
        Err(e) => {
            respond_error(request, e);
            return;
        }
    };
    let result = (|| -> Result<Option<Value>> {
        match (method.as_str(), path.as_str()) {
            ("GET", "/v1/status") => Ok(Some(
                json!({"app":"gendock","addresses":app.addresses,"running":true,"server_id":app.receipt.server_id,"gpu_concurrency":1,"client":principal.name,"stopping":app.stop.load(Ordering::Relaxed)}),
            )),
            ("POST", "/v1/shutdown") => {
                if !principal.admin {
                    return Err(error(
                        "forbidden",
                        "Administrator access is required.",
                        "Ask the device administrator to stop the service.",
                        json!({}),
                    ));
                }
                Ok(Some(json!({"stopping":true})))
            }
            ("POST", "/v1/uploads") => Ok(Some(super::store_upload(
                &app.paths,
                &principal.name,
                &body(&mut request, 64 * 1024 * 1024)?,
            )?)),
            ("POST", "/v1/jobs") => {
                let payload=serde_json::from_slice::<super::Submission>(&body(&mut request,1024*1024)?).map_err(|_|error("invalid_submission","Task JSON is invalid or includes unknown fields.","Provide a supported task and upload IDs; server filesystem paths are not accepted.",json!({})))?;
                Ok(Some(super::submit(&app, &principal, payload)?))
            }
            ("GET", "/v1/jobs") => Ok(Some(json!(
                store
                    .visible_jobs(&principal.name, principal.admin)?
                    .iter()
                    .map(public_job)
                    .collect::<Vec<_>>()
            ))),
            _ => Ok(None),
        }
    })();
    match result {
        Err(error) => {
            respond_error(request, error);
            return;
        }
        Ok(Some(data)) => {
            json_response(request, 200, output::success(data));
            if method == "POST" && path == "/v1/shutdown" {
                app.stop.store(true, Ordering::Relaxed);
            }
            return;
        }
        Ok(None) => {}
    }
    let parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
    if parts.len() < 3 || parts.len() > 4 || parts[0] != "v1" || parts[1] != "jobs" {
        respond_error(
            request,
            error(
                "route_not_found",
                "API route not found.",
                "Use a documented /v1 endpoint.",
                json!({}),
            ),
        );
        return;
    }
    let id = match parts[2].parse::<i64>() {
        Ok(id) if id > 0 => id,
        _ => {
            respond_error(
                request,
                error(
                    "job_not_found",
                    "The job does not exist.",
                    "Use a valid job ID.",
                    json!({}),
                ),
            );
            return;
        }
    };
    let job = match find_job(&store, id, &principal) {
        Ok(job) => job,
        Err(error) => {
            respond_error(request, error);
            return;
        }
    };
    match (method.as_str(), parts.get(3).copied()) {
        ("GET", None) => json_response(request, 200, output::success(public_job(&job))),
        ("POST", Some("cancel")) => {
            match store.cancel_service(id, &principal.name, principal.admin) {
                Ok(Some(job)) => json_response(request, 200, output::success(public_job(&job))),
                Ok(None) => respond_error(
                    request,
                    error(
                        "job_not_found",
                        "The job is unavailable.",
                        "Use a valid owned job ID.",
                        json!({}),
                    ),
                ),
                Err(error) => respond_error(request, error),
            }
        }
        ("GET", Some("events")) => events(request, app, id, principal),
        ("GET", Some("file")) => {
            let result = (|| -> Result<std::fs::File> {
                if job["status"] != "succeeded" {
                    return Err(error(
                        "job_not_complete",
                        "The task has no completed output.",
                        "Wait for success before fetching files.",
                        json!({"job_id":id}),
                    ));
                }
                let path = store.asset_file(id)?.ok_or_else(|| {
                    error(
                        "artifact_not_found",
                        "The output file is unavailable.",
                        "Ask the device administrator to inspect the task.",
                        json!({"job_id":id}),
                    )
                })?;
                let path = std::path::PathBuf::from(path).canonicalize()?;
                let root = app.paths.data.join("assets").canonicalize()?;
                anyhow::ensure!(path.starts_with(root), "output is outside managed assets");
                Ok(std::fs::File::open(path)?)
            })();
            match result {
                Ok(file) => {
                    let _ = request.respond(
                        Response::from_file(file)
                            .with_header(header("Content-Type", "image/png"))
                            .with_header(header(
                                "Content-Disposition",
                                &format!("attachment; filename=\"job_{id}.png\""),
                            )),
                    );
                }
                Err(error) => respond_error(request, error),
            }
        }
        _ => respond_error(
            request,
            error(
                "route_not_found",
                "API route not found.",
                "Use a documented endpoint and method.",
                json!({}),
            ),
        ),
    }
}
fn events(request: AccessRequest, app: Arc<App>, id: i64, principal: auth::Principal) {
    let mut writer = request.into_writer();
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nTransfer-Encoding: chunked\r\nX-Request-ID: {}\r\n\r\n",
        writer.request_id()
    );
    if writer
        .write_all(headers.as_bytes())
        .and_then(|_| writer.flush())
        .is_err()
    {
        return;
    }
    let mut previous = String::new();
    let mut heartbeat = Instant::now();
    while !app.stop.load(Ordering::Relaxed) {
        let job =
            match Store::open(&app.paths.database()).and_then(|s| find_job(&s, id, &principal)) {
                Ok(job) => job,
                Err(_) => break,
            };
        let status = job["status"].as_str().unwrap_or("unknown");
        if status != previous {
            let frame = format!(
                "id: {id}-{status}\nevent: job\ndata: {}\n\n",
                output::success(public_job(&job))
            );
            if write_chunk(&mut writer, frame.as_bytes()).is_err() {
                break;
            }
            previous = status.into();
            if ["succeeded", "failed", "cancelled", "interrupted"].contains(&status) {
                break;
            }
        }
        if heartbeat.elapsed() >= Duration::from_secs(1) {
            if write_chunk(&mut writer, b": heartbeat\n\n").is_err() {
                break;
            }
            heartbeat = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = writer.write_all(b"0\r\n\r\n").and_then(|_| writer.flush());
}
fn write_chunk(writer: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    write!(writer, "{:x}\r\n", bytes.len())?;
    writer.write_all(bytes)?;
    writer.write_all(b"\r\n")?;
    writer.flush()
}
