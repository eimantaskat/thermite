use std::str::Utf8Error;

use bytes::Bytes;
use http::response::Parts;
use http::{HeaderMap, StatusCode};
use serde::de::DeserializeOwned;
use thermite_core::{BodyError, Response};

// a response whose body was read whole
#[derive(Debug)]
pub struct TestResponse {
    head: Parts,
    body: Bytes,
}

impl TestResponse {
    pub(super) fn new(head: Parts, body: Bytes) -> Self {
        Self { head, body }
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.head.status
    }

    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    #[must_use]
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    pub fn text(&self) -> Result<&str, Utf8Error> {
        str::from_utf8(self.body())
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(self.body())
    }

    #[must_use]
    pub fn into_response(self) -> Response {
        Response::from_parts(self.head, self.body.into())
    }
}

// a response whose body is read as it is produced, like httpx's `client.stream`, so an endless one can be tested
#[derive(Debug)]
pub struct TestStreamingResponse {
    response: Response,
}

impl TestStreamingResponse {
    pub(super) fn new(response: Response) -> Self {
        Self { response }
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.response.status()
    }

    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        self.response.headers()
    }

    // a chunk as the app's stream produced it; over the wire, chunks may be split or joined
    pub async fn chunk(&mut self) -> Option<Result<Bytes, BodyError>> {
        self.response.body_mut().chunk().await
    }

    #[must_use]
    pub fn into_response(self) -> Response {
        self.response
    }
}
