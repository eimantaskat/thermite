mod connection;
mod handshake;
mod session;
mod socket;

use std::fmt;

use bytes::Bytes;

pub(crate) use connection::{BoxedWebSocketHandler, open};
pub use connection::{WebSocket, WebSocketConnection, WebSocketHandler};
pub(crate) use handshake::{AcceptKey, check_handshake, is_handshake, refusal};
pub(crate) use session::{Session, take_session};
pub(crate) use socket::{CLOSE_TIMEOUT, Shutdown, Socket};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Message {
    Text(String),
    Binary(Bytes),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CloseCode(u16);

impl CloseCode {
    pub const NORMAL: Self = Self(1000);
    pub const GOING_AWAY: Self = Self(1001);
    pub const PROTOCOL_ERROR: Self = Self(1002);
    pub const UNSUPPORTED_DATA: Self = Self(1003);
    // what a close frame without a code reads as, and closing with it sends a frame without one
    pub const NO_STATUS: Self = Self(1005);
    // what a connection lost without a close frame reads as, and closing with it sends none
    pub const ABNORMAL: Self = Self(1006);
    pub const INVALID_DATA: Self = Self(1007);
    pub const POLICY_VIOLATION: Self = Self(1008);
    pub const MESSAGE_TOO_BIG: Self = Self(1009);
    pub const INTERNAL_ERROR: Self = Self(1011);
    pub const SERVICE_RESTART: Self = Self(1012);
    pub const TRY_AGAIN_LATER: Self = Self(1013);

    // the codes a close frame may carry: the registered ones, then those for libraries and apps
    #[must_use]
    pub fn new(code: u16) -> Option<Self> {
        matches!(code, 1000..=1003 | 1007..=1013 | 3000..=4999).then_some(Self(code))
    }

    #[must_use]
    pub fn as_u16(self) -> u16 {
        self.0
    }
}

impl fmt::Display for CloseCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

// like Starlette's, what every call on a connection that has closed answers
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the WebSocket closed with the code {code}")]
pub struct WebSocketDisconnect {
    code: CloseCode,
    reason: String,
}

impl WebSocketDisconnect {
    pub(crate) fn new(code: CloseCode, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }

    pub(crate) fn abnormal() -> Self {
        Self::new(CloseCode::ABNORMAL, "")
    }

    #[must_use]
    pub fn code(&self) -> CloseCode {
        self.code
    }

    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_close_code_is_one_a_close_frame_may_carry() {
        let sendable = [1000, 1003, 1007, 1013, 3000, 4999];
        let refused = [0, 999, 1004, 1005, 1006, 1014, 1015, 2999, 5000];

        for code in sendable {
            assert_eq!(CloseCode::new(code).map(CloseCode::as_u16), Some(code));
        }
        for code in refused {
            assert_eq!(CloseCode::new(code), None, "{code}");
        }
    }

    #[test]
    fn a_disconnect_shows_its_code() {
        let disconnect = WebSocketDisconnect::new(CloseCode::GOING_AWAY, "bye");

        assert_eq!(
            disconnect.to_string(),
            "the WebSocket closed with the code 1001"
        );
        assert_eq!(disconnect.code(), CloseCode::GOING_AWAY);
        assert_eq!(disconnect.reason(), "bye");
    }
}
