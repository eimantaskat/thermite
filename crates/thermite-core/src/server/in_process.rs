use std::sync::Arc;

use bytes::Bytes;
use http::header::CONTENT_LENGTH;
use http::{HeaderValue, Method, StatusCode};

use super::panic::call_catching_panics;
use super::{DEFAULT_BODY_LIMIT, ServerError, content_too_large};
use crate::handler::{Request, RequestHandler, Response};

// answers as the server does, without a socket, so tests can call a handler directly
#[derive(Debug)]
pub struct InProcessServer<Handler> {
    handler: Arc<Handler>,
    body_limit: usize,
    panics: Panics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panics {
    // the test fails with the handler's own message, as FastAPI's raise_server_exceptions does
    Reach,
    Answer500,
}

impl<Handler: RequestHandler> InProcessServer<Handler> {
    pub async fn start(handler: Handler) -> Result<Self, ServerError> {
        handler.startup().await.map_err(ServerError::Startup)?;
        Ok(Self {
            handler: Arc::new(handler),
            body_limit: DEFAULT_BODY_LIMIT,
            panics: Panics::Reach,
        })
    }

    #[must_use]
    pub fn with_body_limit(mut self, bytes: usize) -> Self {
        self.body_limit = bytes;
        self
    }

    #[must_use]
    pub fn catch_panics(mut self) -> Self {
        self.panics = Panics::Answer500;
        self
    }

    pub async fn send(&self, request: Request) -> Response {
        let head = request.method() == Method::HEAD;
        let mut response = if request.body().len() > self.body_limit {
            content_too_large()
        } else {
            match self.panics {
                Panics::Reach => self.handler.call(request).await,
                Panics::Answer500 => call_catching_panics(&self.handler, request).await,
            }
        };
        frame_like_hyper(&mut response, head);
        response
    }

    pub async fn stop(self) {
        self.handler.shutdown().await;
    }
}

// what hyper adds to or removes from a response on the wire, other than the date
fn frame_like_hyper(response: &mut Response, head: bool) {
    let status = response.status();
    let bodiless = status.is_informational()
        || status == StatusCode::NO_CONTENT
        || status == StatusCode::NOT_MODIFIED;
    if bodiless {
        response.headers_mut().remove(CONTENT_LENGTH);
    } else if !response.headers().contains_key(CONTENT_LENGTH) {
        let length = HeaderValue::from(response.body().len());
        response.headers_mut().insert(CONTENT_LENGTH, length);
    }
    if head || bodiless {
        *response.body_mut() = Bytes::new();
    }
}
