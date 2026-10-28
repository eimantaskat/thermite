use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use thermite::{App, Server, ServerError};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::timeout;

pub type TestResult = Result<(), Box<dyn Error>>;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

pub struct RunningServer {
    address: SocketAddr,
    shutdown: Arc<Notify>,
    task: JoinHandle<Result<(), ServerError>>,
}

impl RunningServer {
    pub async fn start(app: App) -> Result<Self, ServerError> {
        let server = Server::bind("127.0.0.1:0").await?;
        let address = server.local_address()?;
        let shutdown = Arc::new(Notify::new());
        let task = tokio::spawn(server.serve_with_shutdown(app, {
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
