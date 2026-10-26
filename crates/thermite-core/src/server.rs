use std::convert::Infallible;
use std::fmt::Display;
use std::future::{Future, pending};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::StatusCode;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::task::{JoinError, JoinSet};

use crate::handler::{RequestHandler, Response};

const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServerError {
    #[error("could not bind to {address}")]
    Bind { address: String, source: io::Error },
    #[error("could not read the bound address")]
    LocalAddress(#[source] io::Error),
}

#[derive(Debug)]
pub struct Server {
    listener: TcpListener,
    shutdown_timeout: Duration,
    header_read_timeout: Duration,
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

    pub fn local_address(&self) -> Result<SocketAddr, ServerError> {
        self.listener
            .local_addr()
            .map_err(ServerError::LocalAddress)
    }

    pub async fn serve(self, handler: impl RequestHandler) {
        self.serve_with_shutdown(handler, pending()).await;
    }

    pub async fn serve_with_shutdown(
        self,
        handler: impl RequestHandler,
        shutdown: impl Future<Output = ()>,
    ) {
        let handler = Arc::new(handler);
        let mut builder = http1::Builder::new();
        builder
            .timer(TokioTimer::new())
            .header_read_timeout(self.header_read_timeout);
        let graceful = GracefulShutdown::new();
        let mut connections = JoinSet::new();
        let mut shutdown = std::pin::pin!(shutdown);

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
    }
}

fn serve_connection<Handler: RequestHandler>(
    builder: &http1::Builder,
    graceful: &GracefulShutdown,
    stream: TcpStream,
    peer_address: SocketAddr,
    handler: Arc<Handler>,
) -> impl Future<Output = ()> + Send + 'static {
    let service = service_fn(move |request| {
        let handler = Arc::clone(&handler);
        async move { Ok::<_, Infallible>(handle_request(handler.as_ref(), request).await) }
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
    handler: &impl RequestHandler,
    request: http::Request<Incoming>,
) -> http::Response<Full<Bytes>> {
    let (parts, body) = request.into_parts();
    let response = match body.collect().await {
        Ok(collected) => {
            let request = http::Request::from_parts(parts, collected.to_bytes());
            handler.call(request).await
        }
        Err(error) => {
            tracing::debug!(%error, "failed to read the request body");
            status_response(StatusCode::BAD_REQUEST)
        }
    };
    response.map(Full::new)
}

fn status_response(status: StatusCode) -> Response {
    let mut response = Response::new(Bytes::new());
    *response.status_mut() = status;
    response
}
