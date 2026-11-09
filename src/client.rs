//! Blocking CLI client. Remote workflows never inspect local model installations.
use crate::{
    image,
    paths::Paths,
    server::{self, Receipt, Submission},
};
use anyhow::Result;
use reqwest::{
    Url,
    blocking::{Client, RequestBuilder, Response},
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct ApiError(pub Value);
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "remote operation failed")
    }
}
impl std::error::Error for ApiError {}
pub struct Connection {
    base: Url,
    token: String,
    http: Client,
}
impl Connection {
    pub fn new(url: &str, token: String) -> Result<Self> {
        let base = Url::parse(url).map_err(|_| {
            server::error(
                "invalid_server_url",
                "Service URL is invalid.",
                "Use an HTTP or HTTPS origin.",
                json!({}),
            )
        })?;
        if !["http", "https"].contains(&base.scheme())
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
            || token.chars().any(char::is_control)
        {
            return Err(server::error(
                "invalid_connection",
                "Service URL or token is invalid.",
                "Use an origin URL and set GENDOCK_TOKEN.",
                json!({}),
            ));
        }
        Ok(Self {
            base,
            token,
            http: Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(15))
                .build()?,
        })
    }
    pub fn local_from(paths: &Paths, receipt: &Receipt) -> Result<Self> {
        Self::new(&receipt.url, server::auth::token(paths, "admin")?)
    }
    fn request(&self, method: reqwest::Method, path: &str) -> Result<RequestBuilder> {
        let request = self.http.request(method, self.base.join(path)?);
        Ok(if self.token.is_empty() {
            request
        } else {
            request.bearer_auth(&self.token)
        })
    }
    fn send(&self, request: RequestBuilder) -> Result<Response> {
        request.send().map_err(|_| {
            server::error(
                "server_unreachable",
                "The service could not be reached.",
                "Check its address, network and server status; do not resubmit tasks blindly.",
                json!({"url":self.base.as_str()}),
            )
        })
    }
    fn decode(&self, response: Response) -> Result<Value> {
        let mut bytes = Vec::new();
        response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
            server::error(
                "invalid_server_response",
                "The service did not return valid JSON.",
                "Check the service URL and compatibility.",
                json!({}),
            )
        })?;
        if value["ok"] == false {
            return Err(ApiError(value).into());
        }
        if value["ok"] != true || value["schema_version"] != 1 {
            return Err(server::error(
                "invalid_server_response",
                "The response protocol is incompatible.",
                "Use a compatible gendock service.",
                json!({}),
            ));
        }
        Ok(value["data"].clone())
    }
    pub fn get(&self, path: &str) -> Result<Value> {
        self.decode(self.send(self.request(reqwest::Method::GET, path)?)?)
    }
    pub fn status(&self) -> Result<Value> {
        self.get("/v1/status")
    }
    pub fn shutdown(&self) -> Result<Value> {
        self.decode(self.send(self.request(reqwest::Method::POST, "/v1/shutdown")?)?)
    }
    pub fn submit(&self, payload: &Submission) -> Result<Value> {
        self.decode(
            self.send(
                self.request(reqwest::Method::POST, "/v1/jobs")?
                    .json(payload),
            )?,
        )
    }
    pub fn upload(&self, path: &Path) -> Result<String> {
        image::validate_input(path)?;
        let bytes = std::fs::read(path)?;
        let data = self.decode(
            self.send(
                self.request(reqwest::Method::POST, "/v1/uploads")?
                    .header("Content-Type", "application/octet-stream")
                    .body(bytes),
            )?,
        )?;
        data["upload_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| {
                server::error(
                    "invalid_server_response",
                    "The upload response omitted its ID.",
                    "Check server compatibility.",
                    json!({}),
                )
            })
    }
    pub fn info(&self, id: i64) -> Result<Value> {
        self.get(&format!("/v1/jobs/{id}"))
    }
    pub fn jobs(&self) -> Result<Value> {
        self.get("/v1/jobs")
    }
    pub fn cancel(&self, id: i64) -> Result<Value> {
        self.decode(
            self.send(self.request(reqwest::Method::POST, &format!("/v1/jobs/{id}/cancel"))?)?,
        )
    }
    pub fn wait(&self, id: i64, timeout: u64) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(timeout);
        let initial = self.info(id)?;
        if terminal(&initial) {
            return Ok(initial);
        }
        // SSE is the notification path; persisted polling covers disconnect/restart.
        if let Ok(request) = self.request(reqwest::Method::GET, &format!("/v1/jobs/{id}/events"))
            && let Ok(response) =
                self.send(request.timeout(Duration::from_secs(timeout.saturating_add(5))))
            && response.status().is_success()
        {
            let mut reader = std::io::BufReader::new(response);
            let mut line = String::new();
            while Instant::now() < deadline {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if let Some(data) = line.strip_prefix("data: ")
                    && let Ok(value) = serde_json::from_str::<Value>(data)
                    && terminal(&value["data"])
                {
                    return Ok(value["data"].clone());
                }
            }
        }
        while Instant::now() < deadline {
            if let Ok(job) = self.info(id)
                && terminal(&job)
            {
                return Ok(job);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err(server::error(
            "job_wait_timeout",
            "The task has not reached a terminal state before the timeout.",
            "Wait again using the same job ID; do not submit a duplicate task.",
            json!({"job_id":id}),
        ))
    }
    pub fn bytes(&self, id: i64) -> Result<Vec<u8>> {
        let job = self.info(id)?;
        if job["status"] != "succeeded" {
            return Err(server::error(
                "job_not_complete",
                "The job has no completed output.",
                "Wait for success before fetching.",
                json!({"job_id":id}),
            ));
        }
        let response =
            self.send(self.request(reqwest::Method::GET, &format!("/v1/jobs/{id}/file"))?)?;
        if !response.status().is_success() {
            return self.decode(response).map(|_| Vec::new());
        }
        let mut bytes = Vec::new();
        response
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        use sha2::{Digest, Sha256};
        if bytes.len() > 32 * 1024 * 1024
            || format!("{:x}", Sha256::digest(&bytes))
                != job["result"]["asset"]["sha256"].as_str().unwrap_or("")
        {
            return Err(server::error(
                "artifact_integrity_failed",
                "The downloaded output failed verification.",
                "Retry fetching the same job; do not regenerate it.",
                json!({"job_id":id}),
            ));
        }
        Ok(bytes)
    }
    pub fn fetch(&self, id: i64, directory: &Path) -> Result<Value> {
        let directory = if directory.is_absolute() {
            directory.to_owned()
        } else {
            std::env::current_dir()?.join(directory)
        };
        std::fs::create_dir_all(&directory)?;
        let bytes = self.bytes(id)?;
        let path = image::export_default(&bytes, &directory, &format!("job_{id}"))?;
        Ok(json!({"job_id":id,"path":path,"bytes":bytes.len()}))
    }
}
pub fn terminal(job: &Value) -> bool {
    matches!(
        job["status"].as_str(),
        Some("succeeded" | "failed" | "cancelled" | "interrupted")
    )
}
pub fn ensure_success(job: Value) -> Result<Value> {
    if job["status"] == "succeeded" {
        return Ok(job);
    }
    Err(server::error(
        "job_failed",
        "The task ended without a successful result.",
        "Inspect the existing task before deciding whether to submit another.",
        json!({"job_id":job["id"],"status":job["status"],"error":job["error"]}),
    ))
}
pub fn discover(explicit: Option<&str>) -> Result<Option<Connection>> {
    let paths = Paths::discover()?;
    let config_path = paths.config.join("config.toml");
    let config = if config_path.exists() {
        Some(crate::config::Config::load_or_init(&paths)?)
    } else {
        None
    };
    if let Some(url) = explicit
        .map(str::to_owned)
        .or_else(|| std::env::var("GENDOCK_URL").ok())
        .or_else(|| config.as_ref().and_then(|c| c.remote.endpoint.clone()))
    {
        let token = std::env::var("GENDOCK_TOKEN")
            .ok()
            .or_else(|| config.as_ref().and_then(|c| c.remote.api_key.clone()))
            .unwrap_or_default();
        return Ok(Some(Connection::new(&url, token)?));
    }
    let paths = Paths::discover()?;
    if let Some(receipt) = server::read_receipt(&paths) {
        let connection = if let Ok(token) = std::env::var("GENDOCK_TOKEN") {
            Connection::new(&receipt.url, token)?
        } else {
            Connection::local_from(&paths, &receipt)?
        };
        if connection
            .status()
            .is_ok_and(|status| status["server_id"] == receipt.server_id)
        {
            return Ok(Some(connection));
        }
    }
    Ok(None)
}
pub fn require(explicit: Option<&str>) -> Result<Connection> {
    discover(explicit)?.ok_or_else(|| {
        server::error(
            "server_not_running",
            "No running service was found.",
            "Run gendock server start or provide --url and GENDOCK_TOKEN.",
            json!({}),
        )
    })
}
pub fn image_request(
    connection: &Connection,
    options: image::Options,
    input: Option<PathBuf>,
    asynchronous: bool,
) -> Result<Value> {
    if asynchronous && options.out.is_some() {
        return Err(server::error(
            "invalid_async_output",
            "An asynchronous command cannot export a future file immediately.",
            "Omit --out and use job wait followed by job fetch.",
            json!({}),
        ));
    }
    let cwd = std::env::current_dir()?;
    let config = crate::config::Config::load_or_init(&Paths::discover()?)?;
    let directory = config.output.image_directory(input.is_some(), &cwd);
    let stem = chrono::Local::now().format("image_%m%d_%H%M").to_string();
    let export = options.out.as_ref().map(|path| {
        if path.is_absolute() {
            path.clone()
        } else {
            cwd.join(path)
        }
    });
    if let Some(path) = &export {
        if path.extension().and_then(|s| s.to_str()) != Some("png") {
            return Err(server::error(
                "unsupported_output_format",
                "Only PNG export is supported.",
                "Use a .png output filename.",
                json!({}),
            ));
        }
        if std::fs::symlink_metadata(path).is_ok() {
            return Err(server::error(
                "output_exists",
                "The output path already exists.",
                "Choose another filename.",
                json!({}),
            ));
        }
    }
    let input_upload = input
        .as_deref()
        .map(|path| connection.upload(path))
        .transpose()?;
    let submission = Submission {
        kind: if input.is_some() {
            "image.edit"
        } else {
            "image.generate"
        }
        .into(),
        prompt: options.prompt,
        model: options.model,
        seed: options.seed,
        aspect: options.aspect,
        width: options.width,
        height: options.height,
        steps: options.steps,
        input_upload,
    };
    let submitted = connection.submit(&submission)?;
    if asynchronous {
        return Ok(submitted);
    }
    let id = submitted["job_id"].as_i64().ok_or_else(|| {
        server::error(
            "invalid_server_response",
            "The service omitted the job ID.",
            "Inspect the service response before retrying.",
            json!({}),
        )
    })?;
    let job = ensure_success(connection.wait(id, 3600)?)?;
    let bytes = connection.bytes(id)?;
    let exported = (|| -> Result<PathBuf> {
        if let Some(export) = export {
            image::export_png(&bytes, &export)?;
            Ok(export)
        } else {
            image::export_default(&bytes, &directory, &stem)
        }
    })();
    let path=exported.map_err(|error|server::error("export_failed","Generation succeeded but the client export failed.","Fetch the completed job into another directory; existing files were preserved.",json!({"job_id":id,"status":"succeeded","cause":crate::cli::output::operation_error(&error)["error"]["code"],"fetch_command":["gendock","job","fetch","--id",id.to_string(),"--out","DIRECTORY"]})))?;
    Ok(
        json!({"job_id":id,"status":"succeeded","model":job["result"]["model"],"result":job["result"],"export_path":path}),
    )
}
