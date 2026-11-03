use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http::HeaderValue;
use tokio::sync::{Mutex, oneshot};

use super::handshake::{AcceptKey, accepted, requested_subprotocols};
use super::session::{Decision, HandOver, Session, attach_session};
use super::socket::Socket;
use super::{CloseCode, Message, WebSocketDisconnect};
use crate::arguments::Arguments;
use crate::background::Task;
use crate::handler::{Request, Response};
use crate::response::internal_server_error;

pub trait WebSocketHandler: Send + Sync + 'static {
    fn call(
        &self,
        request: Request,
        arguments: Arguments,
        socket: WebSocket,
    ) -> impl Future<Output = ()> + Send;
}

impl<Function, Output> WebSocketHandler for Function
where
    Function: Fn(Request, Arguments, WebSocket) -> Output + Send + Sync + 'static,
    Output: Future<Output = ()> + Send,
{
    fn call(
        &self,
        request: Request,
        arguments: Arguments,
        socket: WebSocket,
    ) -> impl Future<Output = ()> + Send {
        self(request, arguments, socket)
    }
}

// WebSocketHandler returns impl Future, which rules out dyn WebSocketHandler, so storage goes through this
pub(crate) trait BoxedWebSocketHandler: Send + Sync + 'static {
    fn call_boxed(
        &self,
        request: Request,
        arguments: Arguments,
        socket: WebSocket,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

impl<Handler: WebSocketHandler> BoxedWebSocketHandler for Handler {
    fn call_boxed(
        &self,
        request: Request,
        arguments: Arguments,
        socket: WebSocket,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.call(request, arguments, socket))
    }
}

// runs the handler until it accepts or denies the handshake, which decides what the request answers
pub(crate) async fn open(
    handler: &Arc<dyn BoxedWebSocketHandler>,
    request: Request,
    arguments: Arguments,
    key: AcceptKey,
) -> Response {
    let (decided, mut decision) = oneshot::channel();
    let (hand_over, handed_over) = oneshot::channel();
    let socket = WebSocket {
        subprotocols: requested_subprotocols(&request),
        decided,
        handed_over,
    };
    let handler = Arc::clone(handler);
    let mut running: Task =
        Box::pin(async move { handler.call_boxed(request, arguments, socket).await });
    let decided = tokio::select! {
        biased;
        decided = &mut decision => Some(decided),
        () = &mut running => None,
    };
    match decided {
        Some(Ok(Decision::Accept { subprotocol })) => {
            let mut response = accepted(key, subprotocol);
            attach_session(
                &mut response,
                Session {
                    handler: running,
                    hand_over,
                },
            );
            response
        }
        // the rest of the handler runs before the answer, since whatever the answer carries may be replaced on the way out
        Some(Ok(Decision::Deny(response))) => {
            running.await;
            response
        }
        Some(Err(_)) => {
            tracing::error!("a WebSocket handler dropped its socket before deciding the handshake");
            running.await;
            internal_server_error()
        }
        None => {
            if let Ok(Decision::Deny(response)) = decision.try_recv() {
                return response;
            }
            // like uvicorn, when the app returns without accepting
            tracing::error!("a WebSocket handler returned without deciding the handshake");
            internal_server_error()
        }
    }
}

// a WebSocket whose handshake is not answered yet
pub struct WebSocket {
    subprotocols: Vec<String>,
    decided: oneshot::Sender<Decision>,
    handed_over: oneshot::Receiver<HandOver>,
}

impl WebSocket {
    #[must_use]
    pub fn requested_subprotocols(&self) -> &[String] {
        &self.subprotocols
    }

    pub async fn accept(self) -> Result<WebSocketConnection, WebSocketDisconnect> {
        self.accept_with(None).await
    }

    // like Starlette, this does not check that the client asked for it
    pub async fn accept_subprotocol(
        self,
        subprotocol: HeaderValue,
    ) -> Result<WebSocketConnection, WebSocketDisconnect> {
        self.accept_with(Some(subprotocol)).await
    }

    async fn accept_with(
        self,
        subprotocol: Option<HeaderValue>,
    ) -> Result<WebSocketConnection, WebSocketDisconnect> {
        let _ = self.decided.send(Decision::Accept { subprotocol });
        // the client went away before the connection was upgraded
        let HandOver { socket, held } = self
            .handed_over
            .await
            .map_err(|_| WebSocketDisconnect::abnormal())?;
        Ok(WebSocketConnection {
            socket,
            _held: held,
        })
    }

    // like raising HTTPException before accepting, the handshake is answered with the response
    pub fn deny(self, response: Response) {
        let _ = self.decided.send(Decision::Deny(response));
    }
}

impl fmt::Debug for WebSocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebSocket")
            .field("requested_subprotocols", &self.subprotocols)
            .finish_non_exhaustive()
    }
}

pub struct WebSocketConnection {
    // shared with the session, which closes what the handler leaves open
    socket: Arc<Mutex<Socket>>,
    // dropped with the connection, which tells the session the handler is done with it
    _held: oneshot::Sender<()>,
}

impl WebSocketConnection {
    pub async fn receive(&mut self) -> Result<Message, WebSocketDisconnect> {
        self.socket.lock().await.receive().await
    }

    // a binary message closes the connection with 1003, where Starlette would fail on it
    pub async fn receive_text(&mut self) -> Result<String, WebSocketDisconnect> {
        self.socket.lock().await.receive_text().await
    }

    pub async fn receive_bytes(&mut self) -> Result<Bytes, WebSocketDisconnect> {
        self.socket.lock().await.receive_bytes().await
    }

    pub async fn send(&mut self, message: Message) -> Result<(), WebSocketDisconnect> {
        self.socket.lock().await.send(message).await
    }

    pub async fn send_text(&mut self, text: impl Into<String>) -> Result<(), WebSocketDisconnect> {
        self.send(Message::Text(text.into())).await
    }

    pub async fn send_bytes(&mut self, bytes: impl Into<Bytes>) -> Result<(), WebSocketDisconnect> {
        self.send(Message::Binary(bytes.into())).await
    }

    // a reason longer than a close frame holds is cut to fit
    pub async fn close(&mut self, code: CloseCode, reason: &str) -> WebSocketDisconnect {
        self.socket.lock().await.close(code, reason).await
    }
}

impl fmt::Debug for WebSocketConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebSocketConnection")
            .finish_non_exhaustive()
    }
}
