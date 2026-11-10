mod test_multipart;
mod test_request;
mod test_response;
mod test_websocket;

use http::Method;
use thermite_core::{InProcessServer, ServerError};

use crate::app::App;

pub use test_multipart::TestMultipart;
pub use test_request::{TestRequest, TestRequestError};
pub use test_response::{TestResponse, TestStreamingResponse};
pub use test_websocket::{TestWebSocket, TestWebSocketError, TestWebSocketRequest};

// like `with TestClient(app)`: the app starts here, and its resources stop only in stop
#[derive(Debug)]
pub struct TestClient {
    server: InProcessServer<App>,
}

impl TestClient {
    pub async fn start(app: App) -> Result<Self, ServerError> {
        Ok(Self {
            server: InProcessServer::start(app).await?,
        })
    }

    #[must_use]
    pub fn with_body_limit(mut self, bytes: usize) -> Self {
        self.server = self.server.with_body_limit(bytes);
        self
    }

    // answers a panic with the server's 500, instead of failing the test with it
    #[must_use]
    pub fn catch_panics(mut self) -> Self {
        self.server = self.server.catch_panics();
        self
    }

    pub fn request(&self, method: Method, path: impl Into<String>) -> TestRequest<'_> {
        TestRequest::new(&self.server, method, path.into())
    }

    pub fn get(&self, path: impl Into<String>) -> TestRequest<'_> {
        self.request(Method::GET, path)
    }

    pub fn post(&self, path: impl Into<String>) -> TestRequest<'_> {
        self.request(Method::POST, path)
    }

    pub fn put(&self, path: impl Into<String>) -> TestRequest<'_> {
        self.request(Method::PUT, path)
    }

    pub fn patch(&self, path: impl Into<String>) -> TestRequest<'_> {
        self.request(Method::PATCH, path)
    }

    pub fn delete(&self, path: impl Into<String>) -> TestRequest<'_> {
        self.request(Method::DELETE, path)
    }

    pub fn head(&self, path: impl Into<String>) -> TestRequest<'_> {
        self.request(Method::HEAD, path)
    }

    pub fn options(&self, path: impl Into<String>) -> TestRequest<'_> {
        self.request(Method::OPTIONS, path)
    }

    pub fn trace(&self, path: impl Into<String>) -> TestRequest<'_> {
        self.request(Method::TRACE, path)
    }

    // like `websocket_connect`, with the handshake headers Starlette's TestClient sends
    pub fn websocket(&self, path: impl Into<String>) -> TestWebSocketRequest<'_> {
        TestWebSocketRequest::new(self.request(Method::GET, path))
    }

    pub async fn stop(self) {
        self.server.stop().await;
    }
}
