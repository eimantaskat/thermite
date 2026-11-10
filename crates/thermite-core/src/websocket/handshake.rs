use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use http::header::{
    CONNECTION, CONTENT_TYPE, HeaderName, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY,
    SEC_WEBSOCKET_PROTOCOL, SEC_WEBSOCKET_VERSION, UPGRADE,
};
use http::{HeaderMap, HeaderValue, Method, StatusCode, Version};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;

use crate::handler::{Request, Response};

const VERSION: &str = "13";

// the Sec-WebSocket-Accept a response accepting the handshake answers with
#[derive(Debug)]
pub(crate) struct AcceptKey(HeaderValue);

// RFC 6455 refuses an unknown version with 426, and anything else it cannot read with 400
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvalidHandshake {
    // hyper upgrades only HTTP/1.1, as RFC 6455 asks
    HttpVersion,
    Version,
    Key,
}

impl InvalidHandshake {
    pub(crate) fn into_response(self) -> Response {
        match self {
            Self::Version => {
                let mut response = refusal(StatusCode::UPGRADE_REQUIRED);
                response
                    .headers_mut()
                    .insert(SEC_WEBSOCKET_VERSION, HeaderValue::from_static(VERSION));
                response
            }
            Self::HttpVersion | Self::Key => refusal(StatusCode::BAD_REQUEST),
        }
    }
}

// what uvicorn hands to an app as a WebSocket rather than as a request
pub(crate) fn is_handshake(request: &Request) -> bool {
    request.method() == Method::GET
        && has_token(request.headers(), &CONNECTION, "upgrade")
        && has_token(request.headers(), &UPGRADE, "websocket")
}

pub(crate) fn check_handshake(request: &Request) -> Result<AcceptKey, InvalidHandshake> {
    if request.version() != Version::HTTP_11 {
        return Err(InvalidHandshake::HttpVersion);
    }
    let headers = request.headers();
    let mut versions = headers.get_all(SEC_WEBSOCKET_VERSION).iter();
    if !matches!((versions.next(), versions.next()), (Some(version), None) if version == VERSION) {
        return Err(InvalidHandshake::Version);
    }
    let mut keys = headers.get_all(SEC_WEBSOCKET_KEY).iter();
    match (keys.next(), keys.next()) {
        (Some(key), None) if STANDARD.decode(key).is_ok_and(|key| key.len() == 16) => {
            // base64 is always a valid header value
            HeaderValue::from_str(&derive_accept_key(key.as_bytes()))
                .map(AcceptKey)
                .map_err(|_| InvalidHandshake::Key)
        }
        _ => Err(InvalidHandshake::Key),
    }
}

pub(crate) fn requested_subprotocols(request: &Request) -> Vec<String> {
    tokens(request.headers(), &SEC_WEBSOCKET_PROTOCOL)
        .map(str::to_owned)
        .collect()
}

pub(crate) fn accepted(key: AcceptKey, subprotocol: Option<HeaderValue>) -> Response {
    let mut response = Response::default();
    *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    let headers = response.headers_mut();
    headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
    headers.insert(CONNECTION, HeaderValue::from_static("Upgrade"));
    headers.insert(SEC_WEBSOCKET_ACCEPT, key.0);
    if let Some(subprotocol) = subprotocol {
        headers.insert(SEC_WEBSOCKET_PROTOCOL, subprotocol);
    }
    response
}

// like uvicorn's answer to a WebSocket that is closed before it is accepted
pub(crate) fn refusal(status: StatusCode) -> Response {
    let mut response = Response::default();
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

fn has_token(headers: &HeaderMap, name: &HeaderName, token: &str) -> bool {
    tokens(headers, name).any(|found| found.eq_ignore_ascii_case(token))
}

fn tokens<'headers>(
    headers: &'headers HeaderMap,
    name: &HeaderName,
) -> impl Iterator<Item = &'headers str> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

    fn handshake(headers: &[(&str, &str)]) -> Request {
        let mut builder = http::Request::builder().uri("/chat");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(bytes::Bytes::new()).unwrap()
    }

    fn valid() -> Vec<(&'static str, &'static str)> {
        vec![
            ("connection", "keep-alive, Upgrade"),
            ("upgrade", "WebSocket"),
            ("sec-websocket-version", "13"),
            ("sec-websocket-key", KEY),
        ]
    }

    #[test]
    fn a_handshake_is_a_get_asking_to_upgrade_to_websocket() {
        let mut post = handshake(&valid());
        *post.method_mut() = Method::POST;

        assert!(is_handshake(&handshake(&valid())));
        assert!(!is_handshake(&post));
        assert!(!is_handshake(&handshake(&valid()[1..])));
        assert!(!is_handshake(&handshake(&[valid()[0], ("upgrade", "h2c")])));
    }

    #[test]
    fn accepts_the_key_with_the_answer_of_rfc_6455() {
        let key = check_handshake(&handshake(&valid())).unwrap();

        let response = accepted(key, Some(HeaderValue::from_static("chat")));

        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        let headers = response.headers();
        assert_eq!(
            headers[SEC_WEBSOCKET_ACCEPT],
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
        assert_eq!(headers[UPGRADE], "websocket");
        assert_eq!(headers[CONNECTION], "Upgrade");
        assert_eq!(headers[SEC_WEBSOCKET_PROTOCOL], "chat");
    }

    #[test]
    fn refuses_a_handshake_over_another_http_version_than_1_1_with_400() {
        for version in [Version::HTTP_10, Version::HTTP_2] {
            let mut request = handshake(&valid());
            *request.version_mut() = version;

            let refused = check_handshake(&request).unwrap_err();

            assert_eq!(refused, InvalidHandshake::HttpVersion);
            assert_eq!(refused.into_response().status(), StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn refuses_a_version_other_than_13_with_426() {
        for versions in [&["8"][..], &[], &["13", "13"]] {
            let mut request = handshake(&valid()[..2]);
            request
                .headers_mut()
                .insert(SEC_WEBSOCKET_KEY, HeaderValue::from_static(KEY));
            for version in versions {
                request
                    .headers_mut()
                    .append(SEC_WEBSOCKET_VERSION, HeaderValue::from_static(version));
            }

            let response = check_handshake(&request).unwrap_err().into_response();

            assert_eq!(
                response.status(),
                StatusCode::UPGRADE_REQUIRED,
                "{versions:?}"
            );
            assert_eq!(response.headers()[SEC_WEBSOCKET_VERSION], "13");
        }
    }

    #[test]
    fn refuses_a_key_that_is_not_16_bytes_of_base64_with_400() {
        let short = STANDARD.encode([0; 15]);
        for keys in [&["not base64!"][..], &[short.as_str()], &[], &[KEY, KEY]] {
            let mut request = handshake(&valid()[..3]);
            for key in keys {
                request
                    .headers_mut()
                    .append(SEC_WEBSOCKET_KEY, HeaderValue::from_str(key).unwrap());
            }

            let response = check_handshake(&request).unwrap_err().into_response();

            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{keys:?}");
            assert_eq!(response.body().as_bytes().map(bytes::Bytes::len), Some(0));
        }
    }

    #[test]
    fn reads_every_requested_subprotocol() {
        let request = handshake(&[
            ("sec-websocket-protocol", "graphql-ws, chat"),
            ("sec-websocket-protocol", " json ,"),
        ]);

        assert_eq!(
            requested_subprotocols(&request),
            ["graphql-ws", "chat", "json"]
        );
    }
}
