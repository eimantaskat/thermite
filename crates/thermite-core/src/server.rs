mod access;
mod in_process;
mod panic;
mod response_body;

use std::convert::Infallible;
use std::error::Error;
use std::fmt::Display;
use std::future::{Future, pending};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http::StatusCode;
use http_body_util::{BodyExt, LengthLimitError, Limited};
use hyper::body::{Body, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinError, JoinSet};

use crate::background::{Task, take_background_tasks};
use crate::handler::{RequestHandler, Response};
use crate::http_error::HttpError;
use crate::websocket::{Session, Shutdown, take_session};
use access::AccessLog;
use panic::{call_catching_panics, run_session_catching_panics};
use response_body::ResponseBody;

pub use in_process::{InProcessServer, InProcessWebSocket, WebSocketHandshake};

const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);
// hyper adds the timeout to the current time, which panics on overflow; a longer wait is no limit anyway
const LONGEST_HEADER_READ_TIMEOUT: Duration = Duration::from_hours(365 * 24);
const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServerError {
    #[error("could not bind to {address}")]
    Bind { address: String, source: io::Error },
    #[error("could not serve on the given listener")]
    Listener(#[source] io::Error),
    #[error("could not read the bound address")]
    LocalAddress(#[source] io::Error),
    #[error("the app failed to start")]
    Startup(#[source] Box<dyn Error + Send + Sync>),
}

#[derive(Debug)]
pub struct Server {
    listener: TcpListener,
    shutdown_timeout: Duration,
    header_read_timeout: Option<Duration>,
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
        Ok(Self::on(listener))
    }

    // for a listener made elsewhere, such as one shared between processes
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)] // tokio panics outside a runtime, which being async rules out
    pub async fn from_std(listener: std::net::TcpListener) -> Result<Self, ServerError> {
        listener
            .set_nonblocking(true)
            .map_err(ServerError::Listener)?;
        let listener = TcpListener::from_std(listener).map_err(ServerError::Listener)?;
        Ok(Self::on(listener))
    }

    fn on(listener: TcpListener) -> Self {
        Self {
            listener,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
            header_read_timeout: Some(DEFAULT_HEADER_READ_TIMEOUT),
            body_limit: DEFAULT_BODY_LIMIT,
        }
    }

    #[must_use]
    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    // it also closes keep-alive connections left idle that long; None turns it off, as behind a gateway
    #[must_use]
    pub fn with_header_read_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.header_read_timeout = timeout
            .into()
            .filter(|timeout| *timeout <= LONGEST_HEADER_READ_TIMEOUT);
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
        let (stopping, stopped) = watch::channel(false);
        let mut connections = JoinSet::new();
        let mut shutdown = std::pin::pin!(shutdown_requested());

        loop {
            tokio::select! {
                accepted = self.listener.accept() => match accepted {
                    Ok((stream, peer_address)) => {
                        // a second write before a read waits about 40 ms for the delayed ack on Linux
                        if let Err(error) = stream.set_nodelay(true) {
                            tracing::debug!(%error, "failed to turn off Nagle's algorithm");
                        }
                        connections.spawn(serve_connection(
                            &builder,
                            Shutdown::new(stopped.clone()),
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
        // connections finish the request they are on, and WebSockets close with 1012
        stopping.send_replace(true);
        let stopping = async {
            // a connection still runs the background tasks of its responses
            let finishing = async {
                while let Some(finished) = connections.join_next().await {
                    log_if_failed(finished);
                }
            };
            if tokio::time::timeout(self.shutdown_timeout, finishing)
                .await
                .is_err()
            {
                tracing::warn!(
                    remaining = connections.len(),
                    "shutdown timeout passed, closing the remaining connections and their tasks"
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
    shutdown: Shutdown,
    stream: TcpStream,
    peer_address: SocketAddr,
    handler: Arc<Handler>,
    body_limit: usize,
) -> impl Future<Output = ()> + Send + 'static {
    let (sent, mut received) = mpsc::unbounded_channel::<Task>();
    let session_shutdown = shutdown.clone();
    let service = service_fn(move |mut request: http::Request<Incoming>| {
        let handler = Arc::clone(&handler);
        let sent = sent.clone();
        let shutdown = session_shutdown.clone();
        async move {
            let access = AccessLog::start(peer_address, &request);
            let upgrade = request.extensions_mut().remove::<OnUpgrade>();
            let mut response = handle_request(&handler, request, body_limit).await;
            if let Some(access) = access {
                access.finish(response.status());
            }
            if let (Some(session), Some(upgrade)) = (take_session(&mut response), upgrade) {
                let _ = sent.send(Box::pin(run_session(session, upgrade, shutdown)));
            }
            let tasks = take_background_tasks(&mut response);
            Ok::<_, Infallible>(response.map(|body| ResponseBody::new(body, tasks, sent)))
        }
    });
    let connection = builder
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades();
    let serving = async move {
        let mut connection = std::pin::pin!(connection);
        let mut shutdown = shutdown;
        let finished = tokio::select! {
            finished = connection.as_mut() => finished,
            () = shutdown.requested() => {
                connection.as_mut().graceful_shutdown();
                connection.await
            }
        };
        if let Err(error) = finished {
            tracing::debug!(%error, %peer_address, "connection ended with an error");
        }
    };
    // one response's tasks run in order, beside those of the others and the WebSockets; dropping the set aborts them
    let running = async move {
        let mut running = JoinSet::new();
        loop {
            tokio::select! {
                // the channel closes once the connection and the last of its responses are dropped
                received_task = received.recv() => match received_task {
                    Some(task) => {
                        running.spawn(task);
                    }
                    None => break,
                },
                Some(_) = running.join_next() => {}
            }
        }
        while running.join_next().await.is_some() {}
    };

    async move {
        tokio::join!(serving, running);
    }
}

async fn run_session(session: Session, upgrade: OnUpgrade, shutdown: Shutdown) {
    let upgraded = match upgrade.await {
        Ok(upgraded) => upgraded,
        // the client went away before the 101 was sent
        Err(error) => {
            tracing::debug!(%error, "a WebSocket connection was not upgraded");
            return;
        }
    };
    session
        .run(
            Box::new(TokioIo::new(upgraded)),
            shutdown,
            run_session_catching_panics,
        )
        .await;
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
) -> Response {
    let (parts, body) = request.into_parts();
    // a declared length is refused before any of the body is read
    if body.size_hint().lower() > body_limit as u64 {
        return content_too_large();
    }
    match Limited::new(body, body_limit).collect().await {
        Ok(collected) => {
            let request = http::Request::from_parts(parts, collected.to_bytes());
            call_catching_panics(handler, request).await
        }
        Err(error) if error.is::<LengthLimitError>() => content_too_large(),
        Err(error) => {
            tracing::debug!(%error, "failed to read the request body");
            status_response(StatusCode::BAD_REQUEST)
        }
    }
}

fn content_too_large() -> Response {
    HttpError::new(StatusCode::PAYLOAD_TOO_LARGE, "Content Too Large").into_response()
}

fn status_response(status: StatusCode) -> Response {
    let mut response = Response::default();
    *response.status_mut() = status;
    response
}
