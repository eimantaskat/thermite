use std::net::SocketAddr;
use std::time::Instant;

use http::{Method, StatusCode, Uri, Version};
use tracing::Level;

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
    // None when no subscriber wants the event, so the copies and the clock are skipped too
    pub(super) fn start<Body>(client: SocketAddr, request: &http::Request<Body>) -> Option<Self> {
        let wanted = tracing::enabled!(
            target: TARGET,
            Level::INFO,
            { client, method, path, version, status, duration_ms }
        );
        wanted.then(|| Self {
            client,
            method: request.method().clone(),
            uri: request.uri().clone(),
            version: request.version(),
            started: Instant::now(),
        })
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
