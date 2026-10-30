mod access;
mod panic;

use std::convert::Infallible;
use std::error::Error;
use std::fmt::Display;
use std::future::{Future, pending};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::StatusCode;
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::{Body, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::task::{JoinError, JoinSet};

use crate::handler::{RequestHandler, Response};
use crate::http_error::HttpError;
use access::AccessLog;
use panic::call_catching_panics;

const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServerError {
    #[error("could not bind to {address}")]
    Bind { address: String, source: io::Error },
    #[error("could not read the bound address")]
    LocalAddress(#[source] io::Error),
    #[error("the app failed to start")]
    Startup(#[source] Box<dyn Error + Send + Sync>),
}

#[derive(Debug)]
pub struct Server {
    listener: TcpListener,
    shutdown_timeout: Duration,
    header_read_timeout: Duration,
    body_limit: usize,
}

impl Server {
    pub async fn bind(address: impl ToSocketAddrs + Display) -> Result<Self, ServerError> {
        let shown_address = address.to_string();
        let listener = TcpListener::bind(address)
            .await
            .map_err(|source| ServerError::Bind {
                address: shown_address,
                source,
            })?;
        Ok(Self {
            listener,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
            header_read_timeout: DEFAULT_HEADER_READ_TIMEOUT,
            body_limit: DEFAULT_BODY_LIMIT,
        })
    }

    #[must_use]
    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_header_read_timeout(mut self, timeout: Duration) -> Self {
        self.header_read_timeout = timeout;
        self
    }

    // in bytes; it also bounds the JSON parsing and validation that run on the async worker threads
    #[must_use]
    pub fn with_body_limit(mut self, bytes: usize) -> Self {
        self.body_limit = bytes;
        self
    }

    pub fn local_address(&self) -> Result<SocketAddr, ServerError> {
        self.listener
            .local_addr()
            .map_err(ServerError::LocalAddress)
    }

    pub async fn serve(self, handler: impl RequestHandler) -> Result<(), ServerError> {
        self.serve_with_shutdown(handler, pending::<()>).await
    }

    // the first shutdown request is graceful, and a second one skips what is left of it, as in uvicorn
    pub async fn serve_with_shutdown<Requested: Future<Output = ()>>(
        self,
        handler: impl RequestHandler,
        mut shutdown_requested: impl FnMut() -> Requested,
    ) -> Result<(), ServerError> {
        handler.startup().await.map_err(ServerError::Startup)?;
        let handler = Arc::new(handler);
        let mut builder = http1::Builder::new();
        builder
            .timer(TokioTimer::new())
            .header_read_timeout(self.header_read_timeout);
        let graceful = GracefulShutdown::new();
        let mut connections = JoinSet::new();
        let mut shutdown = std::pin::pin!(shutdown_requested());

        loop {
            tokio::select! {
                accepted = self.listener.accept() => match accepted {
                    Ok((stream, peer_address)) => {
                        connections.spawn(serve_connection(
                            &builder,
                            &graceful,
                            stream,
                            peer_address,
                            Arc::clone(&handler),
                            self.body_limit,
                        ));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to accept a connection");
                        // errors such as too many open files repeat at once, so wait instead of spinning
                        tokio::select! {
                            () = tokio::time::sleep(ACCEPT_ERROR_BACKOFF) => {}
                            () = &mut shutdown => break,
                        }
                    }
                },
                Some(finished) = connections.join_next() => log_if_failed(finished),
                () = &mut shutdown => break,
            }
        }

        drop(self.listener);
        let stopping = async {
            if tokio::time::timeout(self.shutdown_timeout, graceful.shutdown())
                .await
                .is_err()
            {
                tracing::warn!(
                    remaining = connections.len(),
                    "shutdown timeout passed, closing the remaining connections"
                );
            }
            connections.shutdown().await;
            handler.shutdown().await;
        };
        tokio::select! {
            () = stopping => {}
            () = shutdown_requested() => {
                tracing::warn!("shutdown forced, skipping what was left of it");
            }
        }
        Ok(())
    }
}

fn serve_connection<Handler: RequestHandler>(
    builder: &http1::Builder,
    graceful: &GracefulShutdown,
    stream: TcpStream,
    peer_address: SocketAddr,
    handler: Arc<Handler>,
    body_limit: usize,
) -> impl Future<Output = ()> + Send + 'static {
    let service = service_fn(move |request| {
        let handler = Arc::clone(&handler);
        async move {
            let access = AccessLog::start(peer_address, &request);
            let response = handle_request(&handler, request, body_limit).await;
            access.finish(response.status());
            Ok::<_, Infallible>(response)
        }
    });
    let connection = graceful.watch(builder.serve_connection(TokioIo::new(stream), service));

    async move {
        if let Err(error) = connection.await {
            tracing::debug!(%error, %peer_address, "connection ended with an error");
        }
    }
}

fn log_if_failed(finished: Result<(), JoinError>) {
    if let Err(error) = finished {
        tracing::error!(%error, "a connection task panicked");
    }
}

async fn handle_request(
    handler: &Arc<impl RequestHandler>,
    request: http::Request<Incoming>,
    body_limit: usize,
) -> http::Response<Full<Bytes>> {
    let (parts, body) = request.into_parts();
    // a declared length is refused before any of the body is read
    if body.size_hint().lower() > body_limit as u64 {
        return content_too_large().map(Full::new);
    }
    let response = match Limited::new(body, body_limit).collect().await {
        Ok(collected) => {
            let request = http::Request::from_parts(parts, collected.to_bytes());
            call_catching_panics(handler, request).await
        }
        Err(error) if error.is::<LengthLimitError>() => content_too_large(),
        Err(error) => {
            tracing::debug!(%error, "failed to read the request body");
            status_response(StatusCode::BAD_REQUEST)
        }
    };
    response.map(Full::new)
}

fn content_too_large() -> Response {
    HttpError::new(StatusCode::PAYLOAD_TOO_LARGE, "Content Too Large").into_response()
}

fn status_response(status: StatusCode) -> Response {
    let mut response = Response::new(Bytes::new());
    *response.status_mut() = status;
    response
}
