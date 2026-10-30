use std::net::SocketAddr;
use std::time::Instant;

use http::{Method, StatusCode, Uri, Version};

const TARGET: &str = "thermite::access";

// what uvicorn's access log shows, plus how long the response took
pub(super) struct AccessLog {
    client: SocketAddr,
    method: Method,
    uri: Uri,
    version: Version,
    started: Instant,
}

impl AccessLog {
    pub(super) fn start<Body>(client: SocketAddr, request: &http::Request<Body>) -> Self {
        Self {
            client,
            method: request.method().clone(),
            uri: request.uri().clone(),
            version: request.version(),
            started: Instant::now(),
        }
    }

    pub(super) fn finish(self, status: StatusCode) {
        tracing::info!(
            target: TARGET,
            client = %self.client,
            method = %self.method,
            path = %self.uri,
            version = ?self.version,
            status = status.as_u16(),
            duration_ms = self.started.elapsed().as_secs_f64() * 1000.0,
            "request"
        );
    }
}
