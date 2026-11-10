use std::fmt;
use std::panic::resume_unwind;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::FutureExt;
use http::header::{CONTENT_LENGTH, TRANSFER_ENCODING};
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use tokio::task::{JoinError, JoinHandle};
use tokio_tungstenite::tungstenite::protocol::Role;

use super::panic::{call_catching_panics, run_catching_panics, run_session_catching_panics};
use super::{DEFAULT_BODY_LIMIT, ServerError, content_too_large};
use crate::background::{run_in_order, take_background_tasks};
use crate::body::{Body, BodyError};
use crate::handler::{Request, RequestHandler, Response};
use crate::websocket::{
    CLOSE_TIMEOUT, CloseCode, Message, Shutdown, Socket, WebSocketDisconnect, take_session,
};

mod streamed;

use streamed::Streamed;

// enough for the frames of one message to pass without the two ends waiting on each other often
const PIPE_CAPACITY: usize = 64 * 1024;

// answers as the server does, without a socket, so tests can call a handler directly
#[derive(Debug)]
pub struct InProcessServer<Handler> {
    handler: Arc<Handler>,
    body_limit: usize,
    panics: Panics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panics {
    // the test fails with the handler's own message, as FastAPI's raise_server_exceptions does
    Reach,
    Answer500,
}

#[derive(Debug)]
pub enum WebSocketHandshake {
    Accepted(InProcessWebSocket),
    Denied(Response),
}

impl<Handler: RequestHandler> InProcessServer<Handler> {
    pub async fn start(handler: Handler) -> Result<Self, ServerError> {
        handler.startup().await.map_err(ServerError::Startup)?;
        Ok(Self {
            handler: Arc::new(handler),
            body_limit: DEFAULT_BODY_LIMIT,
            panics: Panics::Reach,
        })
    }

    #[must_use]
    pub fn with_body_limit(mut self, bytes: usize) -> Self {
        self.body_limit = bytes;
        self
    }

    #[must_use]
    pub fn catch_panics(mut self) -> Self {
        self.panics = Panics::Answer500;
        self
    }

    // the body read whole, so a stream that fails gives its error instead of the response
    pub async fn send(&self, request: Request) -> Result<http::Response<Bytes>, BodyError> {
        let (parts, body) = self.send_streaming(request).await.into_parts();
        Ok(http::Response::from_parts(parts, body.into_bytes().await?))
    }

    // the body left to be read chunk by chunk, running the background tasks once it ends
    pub async fn send_streaming(&self, request: Request) -> Response {
        let head = request.method() == Method::HEAD;
        let mut response = self.call(request).await;
        frame_like_hyper(&mut response, head);
        let tasks = take_background_tasks(&mut response);
        let panics = self.panics;
        response.map(|body| Body::from_stream(Streamed::new(body, tasks, panics)))
    }

    // the WebSocket runs over an in-memory pipe, so both ends speak the protocol as on the wire
    pub async fn connect(&self, request: Request) -> WebSocketHandshake {
        let mut response = self.call(request).await;
        let Some(session) = take_session(&mut response) else {
            return WebSocketHandshake::Denied(self.finish(response, false).await);
        };
        let (server_end, client_end) = tokio::io::duplex(PIPE_CAPACITY);
        let panics = self.panics;
        let running = tokio::spawn(session.run(
            Box::new(server_end),
            Shutdown::default(),
            move |handler| async move {
                match panics {
                    Panics::Reach => {
                        handler.await;
                        CloseCode::NORMAL
                    }
                    Panics::Answer500 => run_session_catching_panics(handler).await,
                }
            },
        ));
        let socket = Socket::new(Box::new(client_end), Role::Client, Shutdown::default()).await;
        WebSocketHandshake::Accepted(InProcessWebSocket {
            headers: response.headers().clone(),
            socket: Box::new(socket),
            running: Some(running),
        })
    }

    pub async fn stop(self) {
        self.handler.shutdown().await;
    }

    async fn call(&self, request: Request) -> Response {
        if request.body().len() > self.body_limit {
            return content_too_large();
        }
        match self.panics {
            Panics::Reach => self.handler.call(request).await,
            Panics::Answer500 => call_catching_panics(&self.handler, request).await,
        }
    }

    async fn finish(&self, mut response: Response, head: bool) -> Response {
        frame_like_hyper(&mut response, head);
        // the server runs them after the response is sent, which here is before the response is returned
        let tasks = take_background_tasks(&mut response);
        match self.panics {
            Panics::Reach => run_in_order(tasks).await,
            Panics::Answer500 => run_catching_panics(tasks).await,
        }
        response
    }
}

// the client's end of a WebSocket the app accepted
pub struct InProcessWebSocket {
    headers: HeaderMap,
    socket: Box<Socket>,
    running: Option<JoinHandle<()>>,
}

impl InProcessWebSocket {
    // those of the 101 that accepted the handshake
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub async fn receive(&mut self) -> Result<Message, WebSocketDisconnect> {
        let received = self.socket.receive().await;
        self.reach_panic(&received).await;
        received
    }

    pub async fn receive_text(&mut self) -> Result<String, WebSocketDisconnect> {
        let received = self.socket.receive_text().await;
        self.reach_panic(&received).await;
        received
    }

    pub async fn receive_bytes(&mut self) -> Result<Bytes, WebSocketDisconnect> {
        let received = self.socket.receive_bytes().await;
        self.reach_panic(&received).await;
        received
    }

    pub async fn send(&mut self, message: Message) -> Result<(), WebSocketDisconnect> {
        let sent = self.socket.send(message).await;
        self.reach_panic(&sent).await;
        sent
    }

    // like leaving Starlette's websocket_connect, which waits for the app and raises what it raised
    pub async fn close(&mut self, code: CloseCode, reason: &str) -> WebSocketDisconnect {
        let closed = self.socket.close(code, reason).await;
        if let Some(mut running) = self.running.take() {
            match tokio::time::timeout(CLOSE_TIMEOUT, &mut running).await {
                Ok(finished) => reach(finished),
                Err(_) => self.running = Some(running),
            }
        }
        closed
    }

    // the server's end is dropped without a close frame only when the handler panicked uncaught
    async fn reach_panic<T>(&mut self, outcome: &Result<T, WebSocketDisconnect>) {
        let lost = matches!(outcome, Err(disconnect) if disconnect.code() == CloseCode::ABNORMAL);
        if lost && let Some(running) = self.running.take() {
            reach(running.await);
        }
    }
}

fn reach(finished: Result<(), JoinError>) {
    if let Err(error) = finished
        && error.is_panic()
    {
        resume_unwind(error.into_panic());
    }
}

// a drop cannot wait, so a handler that panics after it is not reached
impl Drop for InProcessWebSocket {
    fn drop(&mut self) {
        if let Some(running) = self.running.take_if(|running| running.is_finished())
            && !std::thread::panicking()
            && let Some(finished) = running.now_or_never()
        {
            reach(finished);
        }
    }
}

impl fmt::Debug for InProcessWebSocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InProcessWebSocket")
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

// what hyper adds to or removes from a response on the wire, other than the date
fn frame_like_hyper(response: &mut Response, head: bool) {
    let status = response.status();
    let bodiless = status.is_informational()
        || status == StatusCode::NO_CONTENT
        || status == StatusCode::NOT_MODIFIED;
    // hyper keeps a transfer encoding only on a response it sends a body with
    if head || bodiless {
        response.headers_mut().remove(TRANSFER_ENCODING);
    }
    let framed = [CONTENT_LENGTH, TRANSFER_ENCODING]
        .iter()
        .any(|header| response.headers().contains_key(header));
    if bodiless {
        response.headers_mut().remove(CONTENT_LENGTH);
    } else if !framed {
        match response.body().as_bytes() {
            Some(body) => {
                let length = HeaderValue::from(body.len());
                response.headers_mut().insert(CONTENT_LENGTH, length);
            }
            // nor does hyper frame a stream it will not send
            None if head => {}
            None => {
                let chunked = HeaderValue::from_static("chunked");
                response.headers_mut().insert(TRANSFER_ENCODING, chunked);
            }
        }
    }
    if head || bodiless {
        *response.body_mut() = Body::empty();
    }
}
