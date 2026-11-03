use std::future::pending;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{self, Error};

use super::{CloseCode, Message, WebSocketDisconnect};

// uvicorn's default ws_max_size
const MESSAGE_LIMIT: usize = 16 * 1024 * 1024;
pub(crate) const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
// a close frame holds 125 bytes, two of them the code
const REASON_LIMIT: usize = 123;

pub(crate) trait Io: AsyncRead + AsyncWrite + Send + Unpin + 'static {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> Io for T {}

// fires once the server starts shutting down; a connection outside a server never sees it
#[derive(Debug, Clone, Default)]
pub(crate) struct Shutdown(Option<watch::Receiver<bool>>);

impl Shutdown {
    pub(crate) fn new(stopping: watch::Receiver<bool>) -> Self {
        Self(Some(stopping))
    }

    pub(crate) async fn requested(&mut self) {
        if let Some(stopping) = &mut self.0
            && stopping.wait_for(|stopping| *stopping).await.is_ok()
        {
            return;
        }
        pending::<()>().await;
    }
}

// what either end of a connection does with it
pub(crate) struct Socket {
    stream: WebSocketStream<Box<dyn Io>>,
    shutdown: Shutdown,
    closed: Option<WebSocketDisconnect>,
}

impl Socket {
    pub(crate) async fn new(io: Box<dyn Io>, role: Role, shutdown: Shutdown) -> Self {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MESSAGE_LIMIT))
            .max_frame_size(Some(MESSAGE_LIMIT));
        Self {
            stream: WebSocketStream::from_raw_socket(io, role, Some(config)).await,
            shutdown,
            closed: None,
        }
    }

    pub(crate) async fn receive(&mut self) -> Result<Message, WebSocketDisconnect> {
        loop {
            if let Some(closed) = &self.closed {
                return Err(closed.clone());
            }
            let received = tokio::select! {
                received = self.stream.next() => received,
                () = self.shutdown.requested() => {
                    // like uvicorn, which closes every WebSocket when it shuts down
                    self.close(CloseCode::SERVICE_RESTART, "").await;
                    continue;
                }
            };
            match received {
                Some(Ok(tungstenite::Message::Text(text))) => {
                    return Ok(Message::Text(text.as_str().to_owned()));
                }
                Some(Ok(tungstenite::Message::Binary(bytes))) => return Ok(Message::Binary(bytes)),
                Some(Ok(tungstenite::Message::Close(frame))) => {
                    // sends the answer tungstenite queued
                    let _ = self.stream.flush().await;
                    self.closed = Some(match frame {
                        Some(frame) => WebSocketDisconnect::new(
                            CloseCode(frame.code.into()),
                            frame.reason.as_str(),
                        ),
                        None => WebSocketDisconnect::new(CloseCode::NO_STATUS, ""),
                    });
                }
                // tungstenite answers pings itself
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    self.fail(&error).await;
                }
                None => self.closed = Some(WebSocketDisconnect::abnormal()),
            }
        }
    }

    pub(crate) async fn receive_text(&mut self) -> Result<String, WebSocketDisconnect> {
        match self.receive().await? {
            Message::Text(text) => Ok(text),
            Message::Binary(_) => Err(self
                .close(CloseCode::UNSUPPORTED_DATA, "expected a text message")
                .await),
        }
    }

    pub(crate) async fn receive_bytes(&mut self) -> Result<bytes::Bytes, WebSocketDisconnect> {
        match self.receive().await? {
            Message::Binary(bytes) => Ok(bytes),
            Message::Text(_) => Err(self
                .close(CloseCode::UNSUPPORTED_DATA, "expected a binary message")
                .await),
        }
    }

    pub(crate) async fn send(&mut self, message: Message) -> Result<(), WebSocketDisconnect> {
        if let Some(closed) = &self.closed {
            return Err(closed.clone());
        }
        let message = match message {
            Message::Text(text) => tungstenite::Message::text(text),
            Message::Binary(bytes) => tungstenite::Message::binary(bytes),
        };
        if let Err(error) = self.stream.send(message).await {
            return Err(self.fail(&error).await);
        }
        Ok(())
    }

    // answers how the connection ended, which is how it had already ended if it had
    pub(crate) async fn close(&mut self, code: CloseCode, reason: &str) -> WebSocketDisconnect {
        if let Some(closed) = &self.closed {
            return closed.clone();
        }
        let reason = &reason[..reason.floor_char_boundary(REASON_LIMIT)];
        let disconnect = WebSocketDisconnect::new(code, reason);
        self.closed = Some(disconnect.clone());
        let frame = match code {
            CloseCode::ABNORMAL => return disconnect,
            CloseCode::NO_STATUS => None,
            code => Some(CloseFrame {
                code: code.0.into(),
                reason: reason.to_owned().into(),
            }),
        };
        let closing = async {
            if self.stream.close(frame).await.is_ok() {
                // what the other end still sends is dropped until it answers the close
                while let Some(Ok(_)) = self.stream.next().await {}
            }
        };
        // an end that stopped reading cannot take the frame, and one that reads may never answer
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, closing).await;
        disconnect
    }

    async fn fail(&mut self, error: &Error) -> WebSocketDisconnect {
        tracing::debug!(%error, "closing a WebSocket that failed");
        let code = match error {
            Error::Capacity(_) => CloseCode::MESSAGE_TOO_BIG,
            Error::Utf8(_) => CloseCode::INVALID_DATA,
            Error::Protocol(ProtocolError::ResetWithoutClosingHandshake) => CloseCode::ABNORMAL,
            Error::Protocol(_) => CloseCode::PROTOCOL_ERROR,
            _ => CloseCode::ABNORMAL,
        };
        self.close(code, "").await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_stream_that_ended_after_its_close_was_lost_reads_as_abnormal() {
        let (server_end, client_end) = tokio::io::duplex(1024);
        let mut socket = Socket::new(Box::new(server_end), Role::Server, Shutdown::default()).await;
        let mut client = WebSocketStream::from_raw_socket(client_end, Role::Client, None).await;

        client.close(None).await.unwrap();
        let closed = socket.receive().await.unwrap_err();
        socket.closed = None;
        let ended = socket.receive().await.unwrap_err();

        assert_eq!(closed.code(), CloseCode::NO_STATUS);
        assert_eq!(ended, WebSocketDisconnect::abnormal());
    }

    #[tokio::test(start_paused = true)]
    async fn closing_gives_up_on_a_peer_that_stopped_reading() {
        let (server_end, _client_end) = tokio::io::duplex(1024);
        let mut socket = Socket::new(Box::new(server_end), Role::Server, Shutdown::default()).await;
        let message = || Message::Binary(bytes::Bytes::from(vec![0; 512]));
        // fills the pipe, which the other end never reads
        while tokio::time::timeout(Duration::from_millis(10), socket.send(message()))
            .await
            .is_ok()
        {}

        let closing =
            tokio::time::timeout(CLOSE_TIMEOUT * 2, socket.close(CloseCode::NORMAL, "")).await;

        assert_eq!(closing.unwrap().code(), CloseCode::NORMAL);
    }
}
