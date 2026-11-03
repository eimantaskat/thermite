mod handler;

use bytes::Bytes;
use http::HeaderValue;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thermite_core::{CloseCode, Message, WebSocketDisconnect};

use crate::response::IntoResponse;

pub(crate) use handler::WebSocketEndpoint;
pub use handler::{WebSocketHandler, WebSocketOutput};

// what a WebSocket handler takes first; the handshake is answered by accepting or denying it
#[derive(Debug)]
pub struct WebSocket(thermite_core::WebSocket);

impl WebSocket {
    #[must_use]
    pub fn requested_subprotocols(&self) -> &[String] {
        self.0.requested_subprotocols()
    }

    pub async fn accept(self) -> Result<WebSocketConnection, WebSocketDisconnect> {
        self.0.accept().await.map(WebSocketConnection)
    }

    pub async fn accept_subprotocol(
        self,
        subprotocol: HeaderValue,
    ) -> Result<WebSocketConnection, WebSocketDisconnect> {
        self.0
            .accept_subprotocol(subprotocol)
            .await
            .map(WebSocketConnection)
    }

    // like raising HTTPException before accepting; uvicorn answers `close()` with 403
    pub fn deny(self, response: impl IntoResponse) {
        self.0.deny(response.into_response());
    }
}

#[derive(Debug)]
pub struct WebSocketConnection(thermite_core::WebSocketConnection);

impl WebSocketConnection {
    pub async fn receive(&mut self) -> Result<Message, WebSocketDisconnect> {
        self.0.receive().await
    }

    pub async fn receive_text(&mut self) -> Result<String, WebSocketDisconnect> {
        self.0.receive_text().await
    }

    pub async fn receive_bytes(&mut self) -> Result<Bytes, WebSocketDisconnect> {
        self.0.receive_bytes().await
    }

    // a message that is not this JSON closes the connection with 1007, where Starlette would fail on it
    pub async fn receive_json<T: DeserializeOwned>(&mut self) -> Result<T, WebSocketDisconnect> {
        let text = self.0.receive_text().await?;
        match serde_json::from_str(&text) {
            Ok(value) => Ok(value),
            Err(error) => Err(self
                .0
                .close(CloseCode::INVALID_DATA, &error.to_string())
                .await),
        }
    }

    pub async fn send(&mut self, message: Message) -> Result<(), WebSocketDisconnect> {
        self.0.send(message).await
    }

    pub async fn send_text(&mut self, text: impl Into<String>) -> Result<(), WebSocketDisconnect> {
        self.0.send_text(text).await
    }

    pub async fn send_bytes(&mut self, bytes: impl Into<Bytes>) -> Result<(), WebSocketDisconnect> {
        self.0.send_bytes(bytes).await
    }

    pub async fn send_json(&mut self, value: &impl Serialize) -> Result<(), WebSocketDisconnect> {
        match serde_json::to_string(value) {
            Ok(text) => self.0.send_text(text).await,
            Err(error) => {
                tracing::error!(%error, "a WebSocket message could not be written as JSON");
                Err(self.0.close(CloseCode::INTERNAL_ERROR, "").await)
            }
        }
    }

    // answers how the connection ended, which is how it had already ended if it had
    pub async fn close(&mut self, code: CloseCode, reason: &str) -> WebSocketDisconnect {
        self.0.close(code, reason).await
    }
}
