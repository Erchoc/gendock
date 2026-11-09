//! Local service lifecycle and shared-device execution.
mod access;
pub mod auth;
mod http;
pub mod launchd;
mod network;
mod worker;
static CONSOLE: AtomicBool = AtomicBool::new(false);
use crate::{config::Config, image, models::ModelError, paths::Paths, store::Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub fn error(
    code: &'static str,
    message: &'static str,
    next: &'static str,
    details: Value,
) -> anyhow::Error {
    ModelError {
        code,
        message,
        retryable: matches!(code, "queue_full" | "server_busy"),
        next_step: next,
        details,
    }
    .into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub pid: u32,
    pub server_id: String,
    pub url: String,
    pub host: String,
    pub port: u16,
}
pub fn receipt_path(paths: &Paths) -> PathBuf {
    paths.state.join("server.json")
}
pub fn read_receipt(paths: &Paths) -> Option<Receipt> {
    serde_json::from_slice(&std::fs::read(receipt_path(paths)).ok()?).ok()
}
pub fn log_path(paths: &Paths) -> PathBuf {
    paths.state.join("logs/server.jsonl")
}
pub fn event(paths: &Paths, name: &str, data: Value) {
    let level = match (name, data["status"].as_str()) {
        ("http_request", _) if data["status_code"].as_u64().unwrap_or(500) >= 500 => "ERROR",
        ("http_request", _)
            if data["status_code"].as_u64().unwrap_or(500) >= 400
                || data["connection_error"] == true =>
        {
            "WARN"
        }
        ("scheduler_error", _) | ("job_finished", Some("failed")) => "ERROR",
        ("job_finished", Some("cancelled" | "interrupted")) => "WARN",
        _ => "INFO",
    };
    if CONSOLE.load(Ordering::Relaxed) {
        use std::io::IsTerminal;
        let color = std::env::var_os("NO_COLOR").is_none()
            && (std::io::stdout().is_terminal()
                || std::env::var("CLICOLOR_FORCE").is_ok_and(|v| v != "0"));
        let label = if color {
            format!(
                "\x1b[{}m{level:5}\x1b[0m",
                match level {
                    "ERROR" => 31,
                    "WARN" => 33,
                    _ => 32,
                }
            )
        } else {
            format!("{level:5}")
        };
        let _ = writeln!(
            std::io::stdout().lock(),
            "{} {label} gendockd {name} {data}",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z")
        );
    }
    if let Some(parent) = log_path(paths).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(paths))
        && fs2::FileExt::lock_exclusive(&file).is_ok()
    {
        let _ = writeln!(
            file,
            "{}",
            json!({"event":name,"level":level,"service":"gendockd","time":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),"data":data})
        );
    }
}
pub struct App {
    pub paths: Paths,
    pub receipt: Receipt,
    pub stop: Arc<AtomicBool>,
    pub max_pending: u32,
    pub addresses: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub kind: String,
    pub prompt: String,
    pub model: Option<String>,
    pub seed: Option<u64>,
    pub aspect: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub steps: Option<u32>,
    pub input_upload: Option<String>,
}
impl Submission {
    pub fn options(&self) -> image::Options {
        image::Options {
            prompt: self.prompt.clone(),
            model: self.model.clone(),
            out: None,
            seed: self.seed,
            aspect: self.aspect.clone(),
            width: self.width,
            height: self.height,
            steps: self.steps,
        }
    }
    pub fn validate(
        &mut self,
        paths: &Paths,
        principal: &auth::Principal,
    ) -> Result<(String, Option<PathBuf>)> {
        if !["image.generate", "image.edit"].contains(&self.kind.as_str())
            || (self.kind == "image.edit") != self.input_upload.is_some()
        {
            return Err(error(
                "invalid_submission",
                "Task type or input upload is invalid.",
                "Use image.generate without input_upload, or image.edit with an owned upload ID.",
                json!({}),
            ));
        }
        let input = self
            .input_upload
            .as_deref()
            .map(|id| upload_path(paths, &principal.name, id))
            .transpose()?;
        let mut options = self.options();
        let revision =
            image::validate_submission(&mut options, input.as_deref(), &self.kind, paths)?;
        self.model = options.model;
        Ok((revision, input))
    }
}
pub fn upload_path(paths: &Paths, owner: &str, id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id).map_err(|_| {
        error(
            "upload_not_found",
            "The upload does not exist.",
            "Upload the image with the same client identity before editing.",
            json!({}),
        )
    })?;
    let path = paths
        .data
        .join("uploads")
        .join(auth::owner_directory(owner))
        .join(id)
        .join("input");
    if !path.is_file() {
        return Err(error(
            "upload_not_found",
            "The upload does not exist.",
            "Upload the image with the same client identity before editing.",
            json!({}),
        ));
    }
    Ok(path)
}
pub fn store_upload(paths: &Paths, owner: &str, bytes: &[u8]) -> Result<Value> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(error(
            "upload_too_large",
            "Image upload exceeds 64 MiB.",
            "Use a smaller image.",
            json!({}),
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let dir = paths
        .data
        .join("uploads")
        .join(auth::owner_directory(owner))
        .join(&id);
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("input");
    std::fs::write(&file, bytes)?;
    match image::validate_input(&file) {
        Ok(metadata) => Ok(json!({"upload_id":id,"image":metadata})),
        Err(error) => {
            let _ = std::fs::remove_dir_all(dir);
            Err(error)
        }
    }
}
pub fn public_job(job: &Value) -> Value {
    let result = &job["result"];
    let summary = if result.is_null() {
        Value::Null
    } else {
        json!({"job_id":job["id"],"status":job["status"],"operation":job["kind"],"model":result["model"],"model_revision":result["model_revision"],"seed":result["seed"],"width":result["width"],"height":result["height"],"steps":result["steps"],"elapsed_ms":result["elapsed_ms"],"asset":{"id":result["asset"]["id"],"media_type":result["asset"]["media_type"],"sha256":result["asset"]["sha256"]}})
    };
    let failure = if job["error"].is_null() {
        Value::Null
    } else {
        json!({"code":job["error"]["code"],"message":job["error"]["message"],"retryable":false,"details":{"job_id":job["id"]},"next_steps":[{"action":"inspect_job","description":"Inspect the task or ask the device administrator to review its private log."}]})
    };
    json!({"id":job["id"],"kind":job["kind"],"status":job["status"],"request":job["request"]["payload"],"cancel_requested":job["request"]["queue"]["cancel_requested"],"result":summary,"error":failure})
}
pub fn submit(app: &App, principal: &auth::Principal, mut payload: Submission) -> Result<Value> {
    let (revision, _) = payload.validate(&app.paths, principal)?;
    let mut stored = serde_json::to_value(&payload)?;
    stored["revision"] = json!(revision);
    let id = Store::open(&app.paths.database())?.submit_service(
        &payload.kind,
        &stored,
        &principal.name,
        app.max_pending,
    )?;
    event(
        &app.paths,
        "job_submitted",
        json!({"job_id":id,"owner":principal.name,"kind":payload.kind}),
    );
    Ok(json!({"job_id":id,"status":"queued","server_id":app.receipt.server_id}))
}
pub fn start(
    host: Option<String>,
    port: Option<u16>,
    daemon: bool,
    register: bool,
) -> Result<Value> {
    let paths = Paths::discover()?;
    let cfg = Config::load_or_init(&paths)?;
    let host = host.unwrap_or(cfg.server.host);
    let port = port.unwrap_or(cfg.server.port);
    let _: std::net::IpAddr = host.parse().context("host must be an IP address")?;
    auth::token(&paths, "admin")?;
    std::fs::create_dir_all(&paths.state)?;
    if register {
        return launchd::register(&paths, &host, port);
    }
    if !daemon {
        return serve(paths, host, port, cfg.server.max_pending_per_user);
    }
    if let Some(receipt) = read_receipt(&paths)
        && crate::client::Connection::local_from(&paths, &receipt).is_ok_and(|c| c.status().is_ok())
    {
        return Err(error(
            "server_already_running",
            "A local service is already running.",
            "Use server status or stop it before starting again.",
            json!({"url":receipt.url}),
        ));
    }
    std::fs::create_dir_all(log_path(&paths).parent().unwrap())?;
    let console_log = paths
        .state
        .join("logs")
        .join(format!("gendockd-{}.log", uuid::Uuid::new_v4()));
    let output = OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(&console_log)?;
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .args([
            "server",
            "start",
            "--host",
            &host,
            "--port",
            &port.to_string(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(receipt) = read_receipt(&paths)
            && receipt.pid == child.id()
            && crate::client::Connection::local_from(&paths, &receipt)
                .is_ok_and(|c| c.status().is_ok())
        {
            return Ok(
                json!({"service":"gendockd","mode":"daemon","addresses":network::discover(&receipt.host,receipt.port),"running":true,"url":receipt.url,"pid":receipt.pid,"server_id":receipt.server_id,"console_log":console_log,"token_file":paths.config.join("server-tokens.json")}),
            );
        }
        if child.try_wait()?.is_some() {
            if let Ok(text) = std::fs::read_to_string(&console_log)
                && let Some(mut failure) = text
                    .lines()
                    .rev()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .find(|v| v["ok"] == false && v["schema_version"] == 1)
            {
                failure["error"]["details"]["log_path"] = json!(console_log);
                return Err(crate::client::ApiError(failure).into());
            }
            return Err(error(
                "server_start_failed",
                "The service exited during startup.",
                "Inspect server logs; check port availability and configuration.",
                json!({"log_path":log_path(&paths)}),
            ));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error(
                "server_start_timeout",
                "The service did not become ready.",
                "Inspect server logs.",
                json!({"log_path":log_path(&paths)}),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn serve(paths: Paths, host: String, port: u16, max_pending: u32) -> Result<Value> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.state.join("server.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| {
        error(
            "server_already_running",
            "A service already owns this data directory.",
            "Use the existing service or another XDG data/state root.",
            json!({}),
        )
    })?;
    Store::open(&paths.database())?;
    let server = tiny_http::Server::http((host.as_str(), port)).map_err(|cause| {
        error(
            "server_bind_failed",
            "The service cannot bind its address.",
            "Choose an unused port, or use gendock server start --port 0 to select an available port.",
            json!({"host":host,"port":port,"reason":cause.to_string(),"suggested_command":["gendock","server","start","--port","0"]}),
        )
    })?;
    let address = server
        .server_addr()
        .to_ip()
        .context("TCP listen address missing")?;
    let local = if address.ip().is_unspecified() {
        if address.is_ipv6() {
            "::1".into()
        } else {
            "127.0.0.1".into()
        }
    } else {
        address.ip().to_string()
    };
    let receipt = Receipt {
        pid: std::process::id(),
        server_id: uuid::Uuid::new_v4().to_string(),
        url: format!(
            "http://{}:{}",
            if local.contains(':') {
                format!("[{local}]")
            } else {
                local
            },
            address.port()
        ),
        host,
        port: address.port(),
    };
    let mut temp = tempfile::NamedTempFile::new_in(&paths.state)?;
    temp.write_all(&serde_json::to_vec(&receipt)?)?;
    temp.as_file().sync_all()?;
    temp.persist(receipt_path(&paths))?;
    let signals = crate::runtime::mflux::Signals::install()?;
    let addresses = network::discover(&receipt.host, receipt.port);
    let app = Arc::new(App {
        paths,
        receipt,
        stop: signals.cancelled.clone(),
        max_pending,
        addresses,
    });
    CONSOLE.store(true, Ordering::Relaxed);
    event(
        &app.paths,
        "server_started",
        json!({"service":"gendockd","pid":app.receipt.pid,"url":app.receipt.url,"port":app.receipt.port,"addresses":app.addresses}),
    );
    let worker = worker::spawn(app.clone());
    let active = Arc::new(AtomicUsize::new(0));
    while !app.stop.load(Ordering::Relaxed) {
        if let Some(request) = server.recv_timeout(Duration::from_millis(100))? {
            if active.fetch_add(1, Ordering::SeqCst) >= 64 {
                active.fetch_sub(1, Ordering::SeqCst);
                http::reject(
                    request,
                    error(
                        "server_busy",
                        "Too many active requests.",
                        "Retry later.",
                        json!({}),
                    ),
                    &app.paths,
                );
                continue;
            }
            let app = app.clone();
            let active = active.clone();
            std::thread::spawn(move || {
                http::handle(request, app);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }
    let _ = worker.join();
    if read_receipt(&app.paths).is_some_and(|r| r.server_id == app.receipt.server_id) {
        let _ = std::fs::remove_file(receipt_path(&app.paths));
    }
    event(
        &app.paths,
        "server_stopped",
        json!({"server_id":app.receipt.server_id}),
    );
    CONSOLE.store(false, Ordering::Relaxed);
    Ok(json!({"running":false}))
}
pub fn logs(paths: &Paths) -> Result<Value> {
    let path = log_path(paths);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let entries = text
        .lines()
        .rev()
        .take(100)
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    Ok(json!({"entries":entries,"log_path":path}))
}
