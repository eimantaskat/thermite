use std::error::Error;
use std::net::SocketAddr;
use std::time::Duration;

use thermite::{App, Server, ServerError};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

pub type TestResult = Result<(), Box<dyn Error>>;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

pub struct RunningServer {
    address: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl RunningServer {
    pub async fn start(app: App) -> Result<Self, ServerError> {
        let server = Server::bind("127.0.0.1:0").await?;
        let address = server.local_address()?;
        let (shutdown, shutdown_signal) = oneshot::channel::<()>();
        let task = tokio::spawn(server.serve_with_shutdown(app, async {
            let _ = shutdown_signal.await;
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
        let _ = self.shutdown.send(());
        timeout(TEST_TIMEOUT, self.task).await??;
        Ok(())
    }
}
