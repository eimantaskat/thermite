use std::error::Error;
use std::future::pending;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderValue, StatusCode};
use thermite_core::{Request, RequestHandler, Response, Server, ServerError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Barrier, Notify, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;

type TestResult = Result<(), Box<dyn Error>>;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

struct RunningServer {
    address: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl RunningServer {
    async fn start(handler: impl RequestHandler) -> Result<Self, ServerError> {
        Self::start_configured(|server| server, handler).await
    }

    async fn start_configured(
        configure: impl FnOnce(Server) -> Server,
        handler: impl RequestHandler,
    ) -> Result<Self, ServerError> {
        let server = configure(Server::bind("127.0.0.1:0").await?);
        let address = server.local_address()?;
        let (shutdown, shutdown_signal) = oneshot::channel::<()>();
        let task = tokio::spawn(server.serve_with_shutdown(handler, async {
            let _ = shutdown_signal.await;
        }));
        Ok(Self {
            address,
            shutdown,
            task,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    async fn stop(self) -> TestResult {
        let _ = self.shutdown.send(());
        timeout(TEST_TIMEOUT, self.task).await??;
        Ok(())
    }
}

fn text_response(status: StatusCode, body: &'static str) -> Response {
    let mut response = Response::new(Bytes::from_static(body.as_bytes()));
    *response.status_mut() = status;
    response
}

#[tokio::test]
async fn returns_the_response_built_by_the_handler() -> TestResult {
    let server = RunningServer::start(|_request: Request| async {
        let mut response = text_response(StatusCode::CREATED, "created");
        response
            .headers_mut()
            .insert("x-thermite", HeaderValue::from_static("hot"));
        response
    })
    .await?;

    let response = reqwest::get(server.url("/")).await?;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["x-thermite"], "hot");
    assert_eq!(response.text().await?, "created");
    server.stop().await
}

#[tokio::test]
async fn passes_method_uri_and_body_to_the_handler() -> TestResult {
    let server = RunningServer::start(|request: Request| async move {
        let body = format!(
            "{} {} {}",
            request.method(),
            request.uri(),
            String::from_utf8_lossy(request.body())
        );
        Response::new(Bytes::from(body))
    })
    .await?;

    let response = reqwest::Client::new()
        .post(server.url("/items?limit=5"))
        .body("hello")
        .send()
        .await?;

    assert_eq!(response.text().await?, "POST /items?limit=5 hello");
    server.stop().await
}

#[tokio::test]
async fn handles_requests_from_several_connections_at_once() -> TestResult {
    let both_arrived = Arc::new(Barrier::new(2));
    let server = RunningServer::start(move |_request: Request| {
        let both_arrived = Arc::clone(&both_arrived);
        async move {
            both_arrived.wait().await;
            text_response(StatusCode::OK, "together")
        }
    })
    .await?;

    let client = reqwest::Client::new();
    let (first, second) = timeout(TEST_TIMEOUT, async {
        tokio::join!(
            client.get(server.url("/")).send(),
            client.get(server.url("/")).send()
        )
    })
    .await?;

    assert_eq!(first?.status(), StatusCode::OK);
    assert_eq!(second?.status(), StatusCode::OK);
    server.stop().await
}

#[tokio::test]
async fn returns_400_when_the_request_body_ends_early() -> TestResult {
    let server =
        RunningServer::start(|_request: Request| async { text_response(StatusCode::OK, "") })
            .await?;

    let response = send_raw(
        server.address,
        b"POST / HTTP/1.1\r\nhost: localhost\r\ncontent-length: 10\r\n\r\nabc",
    )
    .await?;

    assert!(
        response.starts_with("HTTP/1.1 400"),
        "unexpected response: {response}"
    );
    server.stop().await
}

#[tokio::test]
async fn returns_400_when_the_request_is_not_http() -> TestResult {
    let server =
        RunningServer::start(|_request: Request| async { text_response(StatusCode::OK, "") })
            .await?;

    let response = send_raw(server.address, b"not http at all\r\n\r\n").await?;

    assert!(
        response.starts_with("HTTP/1.1 400"),
        "unexpected response: {response}"
    );
    server.stop().await
}

async fn send_raw(address: SocketAddr, request: &[u8]) -> Result<String, Box<dyn Error>> {
    let mut stream = TcpStream::connect(address).await?;
    stream.write_all(request).await?;
    stream.shutdown().await?;
    let mut response = String::new();
    timeout(TEST_TIMEOUT, stream.read_to_string(&mut response)).await??;
    Ok(response)
}

#[tokio::test]
async fn serve_answers_requests_until_its_task_is_aborted() -> TestResult {
    let server = Server::bind("127.0.0.1:0").await?;
    let address = server.local_address()?;
    let task = tokio::spawn(
        server.serve(|_request: Request| async { text_response(StatusCode::OK, "serving") }),
    );

    let response = reqwest::get(format!("http://{address}/")).await?;

    assert_eq!(response.text().await?, "serving");
    task.abort();
    Ok(())
}

#[tokio::test]
async fn finishes_requests_in_flight_before_shutting_down() -> TestResult {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let server = RunningServer::start({
        let started = Arc::clone(&started);
        let release = Arc::clone(&release);
        move |_request: Request| {
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            async move {
                started.notify_one();
                release.notified().await;
                text_response(StatusCode::OK, "finished")
            }
        }
    })
    .await?;

    let request = tokio::spawn(reqwest::get(server.url("/")));
    timeout(TEST_TIMEOUT, started.notified()).await?;
    let _ = server.shutdown.send(());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !server.task.is_finished(),
        "shut down with a request in flight"
    );

    release.notify_one();
    let response = timeout(TEST_TIMEOUT, request).await???;
    assert_eq!(response.text().await?, "finished");
    timeout(TEST_TIMEOUT, server.task).await??;
    Ok(())
}

#[tokio::test]
async fn closes_connections_still_open_when_the_shutdown_timeout_passes() -> TestResult {
    let started = Arc::new(Notify::new());
    let server = RunningServer::start_configured(
        |server| server.with_shutdown_timeout(Duration::from_millis(100)),
        {
            let started = Arc::clone(&started);
            move |_request: Request| {
                let started = Arc::clone(&started);
                async move {
                    started.notify_one();
                    pending::<Response>().await
                }
            }
        },
    )
    .await?;

    let request = tokio::spawn(reqwest::get(server.url("/")));
    timeout(TEST_TIMEOUT, started.notified()).await?;
    server.stop().await?;

    let outcome = timeout(TEST_TIMEOUT, request).await??;
    assert!(
        outcome.is_err(),
        "request finished although its connection was closed"
    );
    Ok(())
}

#[tokio::test]
async fn closes_connections_that_stop_sending_headers() -> TestResult {
    let server = RunningServer::start_configured(
        |server| server.with_header_read_timeout(Duration::from_millis(100)),
        |_request: Request| async { text_response(StatusCode::OK, "") },
    )
    .await?;

    let mut stream = TcpStream::connect(server.address).await?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nhost: localhost\r\n")
        .await?;
    let mut response = Vec::new();
    let read = timeout(TEST_TIMEOUT, stream.read_to_end(&mut response)).await;

    assert!(
        read.is_ok(),
        "connection still open after the header read timeout"
    );
    server.stop().await
}

#[tokio::test]
async fn keeps_serving_after_a_handler_panics() -> TestResult {
    let server = RunningServer::start(|request: Request| async move {
        assert_ne!(request.uri().path(), "/panic", "handler failed");
        text_response(StatusCode::OK, "still serving")
    })
    .await?;

    let failed = reqwest::get(server.url("/panic")).await;
    let response = reqwest::get(server.url("/")).await?;

    assert!(failed.is_err(), "panicking handler produced a response");
    assert_eq!(response.text().await?, "still serving");
    server.stop().await
}

#[tokio::test]
async fn stops_accepting_connections_after_shutting_down() -> TestResult {
    let server =
        RunningServer::start(|_request: Request| async { text_response(StatusCode::OK, "") })
            .await?;
    let address = server.address;

    server.stop().await?;

    let connection = TcpStream::connect(address).await;
    assert!(connection.is_err(), "connected after shutdown");
    Ok(())
}

#[tokio::test]
async fn reports_the_address_when_binding_fails() -> TestResult {
    let taken = Server::bind("127.0.0.1:0").await?;
    let address = taken.local_address()?.to_string();

    let Err(error) = Server::bind(address.as_str()).await else {
        return Err("binding a taken address succeeded".into());
    };

    let ServerError::Bind {
        address: reported,
        source,
    } = &error
    else {
        return Err(format!("unexpected error: {error:?}").into());
    };
    assert_eq!(reported, &address);
    assert_eq!(source.kind(), io::ErrorKind::AddrInUse);
    assert_eq!(error.to_string(), format!("could not bind to {address}"));
    Ok(())
}

#[tokio::test]
async fn reports_the_port_it_bound_to() -> TestResult {
    let server = Server::bind("127.0.0.1:0").await?;

    let address = server.local_address()?;

    assert!(address.ip().is_loopback());
    assert_ne!(address.port(), 0);
    Ok(())
}
