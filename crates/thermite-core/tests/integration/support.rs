use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::StatusCode;
use thermite_core::{RequestHandler, Response, Server, ServerError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::timeout;

pub type TestResult = Result<(), Box<dyn Error>>;

pub const TEST_TIMEOUT: Duration = Duration::from_secs(5);

pub struct RunningServer {
    pub address: SocketAddr,
    // each notification is one shutdown request: the first is graceful, a second forces it
    pub shutdown: Arc<Notify>,
    pub task: JoinHandle<Result<(), ServerError>>,
}

impl RunningServer {
    pub async fn start(handler: impl RequestHandler) -> Result<Self, ServerError> {
        Self::start_configured(|server| server, handler).await
    }

    pub async fn start_configured(
        configure: impl FnOnce(Server) -> Server,
        handler: impl RequestHandler,
    ) -> Result<Self, ServerError> {
        let server = configure(Server::bind("127.0.0.1:0").await?);
        let address = server.local_address()?;
        let shutdown = Arc::new(Notify::new());
        let task = tokio::spawn(server.serve_with_shutdown(handler, {
            let shutdown = Arc::clone(&shutdown);
            move || {
                let shutdown = Arc::clone(&shutdown);
                async move { shutdown.notified().await }
            }
        }));
        Ok(Self {
            address,
            shutdown,
            task,
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    pub async fn stop(self) -> TestResult {
        self.shutdown.notify_one();
        timeout(TEST_TIMEOUT, self.task).await???;
        Ok(())
    }
}

pub async fn send_raw(address: SocketAddr, request: &[u8]) -> Result<String, Box<dyn Error>> {
    let mut stream = TcpStream::connect(address).await?;
    stream.write_all(request).await?;
    stream.shutdown().await?;
    let mut response = String::new();
    timeout(TEST_TIMEOUT, stream.read_to_string(&mut response)).await??;
    Ok(response)
}

pub fn text_response(status: StatusCode, body: &'static str) -> Response {
    let mut response = Response::new(Bytes::from_static(body.as_bytes()));
    *response.status_mut() = status;
    response
}
