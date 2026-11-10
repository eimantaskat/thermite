use std::fmt::Write;

use bytes::Bytes;
use http::header::{CONTENT_LENGTH, CONTENT_TYPE, HOST};
use http::request::Builder;
use http::{HeaderName, HeaderValue, Method};
use serde::Serialize;
use thermite_core::{InProcessServer, Request};

use super::test_response::TestResponse;
use crate::app::App;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TestRequestError {
    #[error("the path `{path}` must start with `/`")]
    MissingLeadingSlash { path: String },
    #[error("the request is not valid HTTP")]
    Http(#[from] http::Error),
    #[error("the body could not be written as JSON")]
    Body(#[from] serde_json::Error),
}

// a mistake is kept until send, so that the calls can still be chained
#[derive(Debug)]
pub struct TestRequest<'client> {
    server: &'client InProcessServer<App>,
    path: String,
    builder: Builder,
    body: Result<Bytes, serde_json::Error>,
    // added at send, so that a content type the test sets is the only one
    body_type: Option<HeaderValue>,
}

impl<'client> TestRequest<'client> {
    pub(super) fn new(server: &'client InProcessServer<App>, method: Method, path: String) -> Self {
        Self {
            server,
            builder: Builder::new().method(method).uri(path.as_str()),
            path,
            body: Ok(Bytes::new()),
            body_type: None,
        }
    }

    #[must_use]
    pub fn header<Name, Value>(mut self, name: Name, value: Value) -> Self
    where
        HeaderName: TryFrom<Name>,
        <HeaderName as TryFrom<Name>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<Value>,
        <HeaderValue as TryFrom<Value>>::Error: Into<http::Error>,
    {
        self.builder = self.builder.header(name, value);
        self
    }

    #[must_use]
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = Ok(body.into());
        self.body_type = None;
        self
    }

    #[must_use]
    pub fn json(mut self, body: &impl Serialize) -> Self {
        self.body = serde_json::to_vec(body).map(Bytes::from);
        self.body_type = Some(HeaderValue::from_static("application/json"));
        self
    }

    // encoded like a browser's form, every byte but letters, digits and `*-._` escaped and spaces as `+`
    #[must_use]
    pub fn form<Name: AsRef<str>, Value: AsRef<str>>(
        mut self,
        pairs: impl IntoIterator<Item = (Name, Value)>,
    ) -> Self {
        let mut encoded = String::new();
        for (name, value) in pairs {
            if !encoded.is_empty() {
                encoded.push('&');
            }
            form_encode(&mut encoded, name.as_ref());
            encoded.push('=');
            form_encode(&mut encoded, value.as_ref());
        }
        self.body = Ok(Bytes::from(encoded));
        self.body_type = Some(HeaderValue::from_static(
            "application/x-www-form-urlencoded",
        ));
        self
    }

    pub async fn send(self) -> Result<TestResponse, TestRequestError> {
        let server = self.server;
        let request = self.build()?;
        Ok(TestResponse::new(server.send(request).await))
    }

    pub(super) fn server(&self) -> &'client InProcessServer<App> {
        self.server
    }

    pub(super) fn build(self) -> Result<Request, TestRequestError> {
        if !self.path.starts_with('/') {
            return Err(TestRequestError::MissingLeadingSlash { path: self.path });
        }
        let body = self.body?;
        let length = body.len();
        let mut request = self.builder.body(body)?;
        // httpx declares an empty body only for the methods that usually carry one
        let declares_length =
            length > 0 || [Method::POST, Method::PUT, Method::PATCH].contains(request.method());
        // what httpx sends without being told, with Starlette's TestClient host
        let headers = request.headers_mut();
        headers
            .entry(HOST)
            .or_insert(HeaderValue::from_static("testserver"));
        if declares_length {
            headers
                .entry(CONTENT_LENGTH)
                .or_insert(HeaderValue::from(length));
        }
        if let Some(body_type) = self.body_type {
            headers.entry(CONTENT_TYPE).or_insert(body_type);
        }
        Ok(request)
    }
}

fn form_encode(encoded: &mut String, text: &str) {
    for byte in text.bytes() {
        match byte {
            b' ' => encoded.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                encoded.push(char::from(byte));
            }
            // writing to a String cannot fail
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
}
