use std::fmt;
use std::sync::Arc;

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use serde_json::{Value, json};

use crate::handler::Response;
use crate::response::{attach_error, json_response};

// FastAPI's HTTPException: answers `{"detail": ...}` with its status and headers
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpError {
    status: StatusCode,
    detail: Value,
    headers: HeaderMap,
}

impl HttpError {
    #[must_use]
    pub fn new(status: StatusCode, detail: impl Into<Value>) -> Self {
        Self {
            status,
            detail: detail.into(),
            headers: HeaderMap::new(),
        }
    }

    #[must_use]
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.append(name, value);
        self
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    #[must_use]
    pub fn detail(&self) -> &Value {
        &self.detail
    }

    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    // the response carries the error too, so that an exception handler can still read it
    #[must_use]
    pub fn into_response(self) -> Response {
        let mut response = json_response(self.status, json!({ "detail": self.detail }).to_string());
        response.headers_mut().extend(self.headers.clone());
        attach_error(&mut response, Arc::new(self));
        response
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(detail) = self.detail.as_str() {
            write!(formatter, "{}: {detail}", self.status)
        } else {
            write!(formatter, "{}: {}", self.status, self.detail)
        }
    }
}

impl std::error::Error for HttpError {}

#[cfg(test)]
mod tests {
    use http::header::{ALLOW, CONTENT_TYPE};

    use super::*;
    use crate::response::response_error;

    #[test]
    fn answers_the_detail_with_its_status_and_headers() {
        let error = HttpError::new(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed")
            .with_header(ALLOW, HeaderValue::from_static("GET"));

        let response = error.clone().into_response();

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
        assert_eq!(response.headers()[ALLOW], "GET");
        assert_eq!(response.body(), r#"{"detail":"Method Not Allowed"}"#);
        let attached = response_error(&response).and_then(|error| error.downcast_ref());
        assert_eq!(attached, Some(&error));
    }

    #[test]
    fn shows_the_status_and_the_detail() {
        let text = HttpError::new(StatusCode::NOT_FOUND, "Item not found");
        let structured = HttpError::new(StatusCode::CONFLICT, json!({"id": 3}));

        assert_eq!(text.to_string(), "404 Not Found: Item not found");
        assert_eq!(structured.to_string(), r#"409 Conflict: {"id":3}"#);
        assert_eq!(structured.detail(), &json!({"id": 3}));
        assert_eq!(structured.status(), StatusCode::CONFLICT);
        assert!(structured.headers().is_empty());
    }
}
