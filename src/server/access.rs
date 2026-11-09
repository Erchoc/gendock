//! Per-request access logging, including rejected requests and streamed responses.
use crate::{paths::Paths, server};
use serde_json::json;
use std::{
    io::{Read, Write},
    ops::{Deref, DerefMut},
    time::Instant,
};
use tiny_http::{Header, Request, Response};

pub(super) struct AccessRequest {
    inner: Option<Request>,
    paths: Paths,
    started: Instant,
    id: String,
    method: String,
    path: String,
    peer: Option<String>,
    pub principal: Option<String>,
    pub code: Option<String>,
    status: u16,
    transport_error: bool,
}
impl AccessRequest {
    pub fn new(request: Request, paths: &Paths) -> Self {
        let path = request.url().split('?').next().unwrap_or("");
        // Preserve unknown routes for diagnosis; omit queries and bound log size.
        // JSON serialization escapes control characters to preserve one-line framing.
        let mut route = path.chars().take(2048).collect::<String>();
        if path.chars().count() > 2048 {
            route.push_str("…[truncated]");
        }
        Self {
            peer: request.remote_addr().map(ToString::to_string),
            method: request.method().as_str().to_owned(),
            path: route,
            inner: Some(request),
            paths: paths.clone(),
            started: Instant::now(),
            id: uuid::Uuid::new_v4().to_string(),
            principal: None,
            code: None,
            status: 500,
            transport_error: false,
        }
    }
    pub fn respond<R: Read>(mut self, response: Response<R>) -> std::io::Result<()> {
        self.status = response.status_code().0;
        let response =
            response.with_header(Header::from_bytes("X-Request-ID", self.id.as_bytes()).unwrap());
        let result = self.inner.take().unwrap().respond(response);
        self.transport_error = result.is_err();
        result
    }
    pub fn into_writer(mut self) -> AccessWriter {
        self.status = 200;
        let inner = self.inner.take().unwrap().into_writer();
        AccessWriter {
            inner,
            access: self,
        }
    }
}
impl Deref for AccessRequest {
    type Target = Request;
    fn deref(&self) -> &Request {
        self.inner.as_ref().unwrap()
    }
}
impl DerefMut for AccessRequest {
    fn deref_mut(&mut self) -> &mut Request {
        self.inner.as_mut().unwrap()
    }
}
impl Drop for AccessRequest {
    fn drop(&mut self) {
        server::event(
            &self.paths,
            "http_request",
            json!({"request_id":self.id,"method":self.method,"path":self.path,"peer":self.peer,"client":self.principal,"status_code":self.status,"duration_ms":self.started.elapsed().as_millis(),"error_code":self.code,"connection_error":self.transport_error}),
        );
    }
}
pub(super) struct AccessWriter {
    inner: Box<dyn Write + Send + 'static>,
    access: AccessRequest,
}
impl AccessWriter {
    pub fn request_id(&self) -> &str {
        &self.access.id
    }
}
impl Write for AccessWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let result = self.inner.write(bytes);
        if result.is_err() {
            self.access.transport_error = true;
        }
        result
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let result = self.inner.flush();
        if result.is_err() {
            self.access.transport_error = true;
        }
        result
    }
}
