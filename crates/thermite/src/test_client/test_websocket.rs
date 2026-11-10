use bytes::Bytes;
use http::header::{CONNECTION, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thermite_core::{
    CloseCode, InProcessWebSocket, Message, WebSocketDisconnect, WebSocketHandshake,
};

use super::test_request::{TestRequest, TestRequestError};
use super::test_response::TestResponse;

// the sample key of RFC 6455, since Starlette's `testserver==` is not 16 bytes of base64
const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TestWebSocketError {
    #[error(transparent)]
    Request(#[from] TestRequestError),
    // like Starlette's WebSocketDenialResponse
    #[error("the handshake was answered with {}", .0.status())]
    Denied(Box<TestResponse>),
}

#[derive(Debug)]
pub struct TestWebSocketRequest<'client> {
    request: TestRequest<'client>,
}

impl<'client> TestWebSocketRequest<'client> {
    pub(super) fn new(request: TestRequest<'client>) -> Self {
        Self { request }
    }

    // a handshake header set here replaces the one the client would send
    #[must_use]
    pub fn header<Name, Value>(mut self, name: Name, value: Value) -> Self
    where
        HeaderName: TryFrom<Name>,
        <HeaderName as TryFrom<Name>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<Value>,
        <HeaderValue as TryFrom<Value>>::Error: Into<http::Error>,
    {
        self.request = self.request.header(name, value);
        self
    }

    pub async fn connect(self) -> Result<TestWebSocket, TestWebSocketError> {
        let server = self.request.server();
        let mut request = self.request.build()?;
        let headers = request.headers_mut();
        for (name, value) in [
            (CONNECTION, "upgrade"),
            (UPGRADE, "websocket"),
            (SEC_WEBSOCKET_VERSION, "13"),
            (SEC_WEBSOCKET_KEY, KEY),
        ] {
            headers
                .entry(name)
                .or_insert(HeaderValue::from_static(value));
        }
        match server.connect(request).await {
            WebSocketHandshake::Accepted(socket) => Ok(TestWebSocket(socket)),
            WebSocketHandshake::Denied(response) => {
                let (head, body) = response.into_parts();
                let body = body.into_bytes().await.map_err(TestRequestError::from)?;
                Err(TestWebSocketError::Denied(Box::new(TestResponse::new(
                    head, body,
                ))))
            }
        }
    }
}

// a panic in the handler fails the test here, unless the client catches panics
#[derive(Debug)]
pub struct TestWebSocket(InProcessWebSocket);

impl TestWebSocket {
    // those of the 101 that accepted the handshake
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        self.0.headers()
    }

    pub async fn receive(&mut self) -> Result<Message, WebSocketDisconnect> {
        self.0.receive().await
    }

    pub async fn receive_text(&mut self) -> Result<String, WebSocketDisconnect> {
        self.0.receive_text().await
    }

    pub async fn receive_bytes(&mut self) -> Result<Bytes, WebSocketDisconnect> {
        self.0.receive_bytes().await
    }

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
        self.0.send(Message::Text(text.into())).await
    }

    pub async fn send_bytes(&mut self, bytes: impl Into<Bytes>) -> Result<(), WebSocketDisconnect> {
        self.0.send(Message::Binary(bytes.into())).await
    }

    // a value serde cannot write closes the connection, as WebSocketConnection does
    pub async fn send_json(&mut self, value: &impl Serialize) -> Result<(), WebSocketDisconnect> {
        match serde_json::to_string(value) {
            Ok(text) => self.send_text(text).await,
            Err(error) => {
                tracing::error!(%error, "a WebSocket message could not be written as JSON");
                Err(self.0.close(CloseCode::INTERNAL_ERROR, "").await)
            }
        }
    }

    pub async fn close(&mut self, code: CloseCode, reason: &str) -> WebSocketDisconnect {
        self.0.close(code, reason).await
    }
}
