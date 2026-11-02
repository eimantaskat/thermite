use std::str::Utf8Error;

use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use serde::de::DeserializeOwned;
use thermite_core::Response;

#[derive(Debug)]
pub struct TestResponse {
    response: Response,
}

impl TestResponse {
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

    #[must_use]
    pub fn body(&self) -> &Bytes {
        self.response.body()
    }

    pub fn text(&self) -> Result<&str, Utf8Error> {
        str::from_utf8(self.body())
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(self.body())
    }

    #[must_use]
    pub fn into_response(self) -> Response {
        self.response
    }
}
