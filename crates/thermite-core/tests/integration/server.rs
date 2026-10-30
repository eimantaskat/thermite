use std::error::Error;
use std::future::{Future, pending, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderValue, StatusCode};
use thermite_core::{Request, RequestHandler, Response, Server, ServerError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Barrier, Notify};
use tokio::time::timeout;

use crate::support::{RunningServer, TEST_TIMEOUT, TestResult, send_raw, text_response};

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
    server.shutdown.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !server.task.is_finished(),
        "shut down with a request in flight"
    );

    release.notify_one();
    let response = timeout(TEST_TIMEOUT, request).await???;
    assert_eq!(response.text().await?, "finished");
    timeout(TEST_TIMEOUT, server.task).await???;
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
async fn answers_500_and_keeps_serving_when_a_handler_panics() -> TestResult {
    let server = RunningServer::start(|request: Request| {
        let path = request.uri().path().to_owned();
        assert_ne!(path, "/panic-before", "failed before the future");
        async move {
            assert_ne!(path, "/panic-inside", "failed inside the future");
            text_response(StatusCode::OK, "still serving")
        }
    })
    .await?;

    for path in ["/panic-before", "/panic-inside"] {
        let failed = reqwest::get(server.url(path)).await?;

        assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR, "{path}");
        assert_eq!(
            failed.headers()["content-type"],
            "text/plain; charset=utf-8"
        );
        assert_eq!(failed.text().await?, "Internal Server Error");
    }
    let response = reqwest::get(server.url("/")).await?;
    assert_eq!(response.text().await?, "still serving");
    server.stop().await
}

fn echoing_body_length() -> impl RequestHandler {
    |request: Request| async move { Response::new(request.body().len().to_string().into()) }
}

#[tokio::test]
async fn answers_413_when_the_body_is_larger_than_the_limit() -> TestResult {
    let server =
        RunningServer::start_configured(|server| server.with_body_limit(4), echoing_body_length())
            .await?;
    let client = reqwest::Client::new();

    let fits = client.post(server.url("/")).body("abcd").send().await?;
    let declared = client.post(server.url("/")).body("abcde").send().await?;
    let streamed = send_raw(
        server.address,
        b"POST / HTTP/1.1\r\nhost: localhost\r\ntransfer-encoding: chunked\r\n\r\n3\r\nabc\r\n3\r\ndef\r\n0\r\n\r\n",
    )
    .await?;

    assert_eq!(fits.text().await?, "4");
    assert_eq!(declared.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(declared.headers()["content-type"], "application/json");
    assert_eq!(declared.text().await?, r#"{"detail":"Content Too Large"}"#);
    assert!(
        streamed.starts_with("HTTP/1.1 413"),
        "unexpected response: {streamed}"
    );
    server.stop().await
}

#[tokio::test]
async fn accepts_bodies_up_to_2_mib_by_default() -> TestResult {
    let server = RunningServer::start(echoing_body_length()).await?;
    let client = reqwest::Client::new();
    let limit = 2 * 1024 * 1024;

    let fits = client
        .post(server.url("/"))
        .body(vec![b'a'; limit])
        .send()
        .await?;
    // refused before the body is read, so a client still sending it would see the connection reset
    let too_large = send_raw(
        server.address,
        format!(
            "POST / HTTP/1.1\r\nhost: localhost\r\ncontent-length: {}\r\n\r\n",
            limit + 1
        )
        .as_bytes(),
    )
    .await?;

    assert_eq!(fits.text().await?, limit.to_string());
    assert!(
        too_large.starts_with("HTTP/1.1 413"),
        "unexpected response: {too_large}"
    );
    server.stop().await
}

#[tokio::test]
async fn releases_its_port_after_shutting_down() -> TestResult {
    let server =
        RunningServer::start(|_request: Request| async { text_response(StatusCode::OK, "") })
            .await?;
    let address = server.address;

    server.stop().await?;

    // binding again proves the listener is gone, while a refused connect takes 2 s on Windows
    Server::bind(address).await?;
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

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<&'static str>>>);

impl Events {
    fn record(&self, event: &'static str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn recorded(&self) -> Vec<&'static str> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// answers at once, except on /hang, which never answers
struct Lifecycle {
    events: Events,
    starts: Result<(), &'static str>,
}

impl RequestHandler for Lifecycle {
    fn call(self: &Arc<Self>, request: Request) -> impl Future<Output = Response> + Send + 'static {
        self.events.record("request");
        let hangs = request.uri().path() == "/hang";
        async move {
            if hangs {
                pending::<()>().await;
            }
            text_response(StatusCode::OK, "")
        }
    }

    async fn startup(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        // long enough for the request sent meanwhile to arrive first, were it accepted
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.events.record("startup");
        self.starts.map_err(Into::into)
    }

    fn shutdown(&self) -> impl Future<Output = ()> + Send {
        self.events.record("shutdown");
        ready(())
    }
}

#[tokio::test]
async fn starts_the_handler_before_the_first_request_and_shuts_it_down_after_the_last() -> TestResult
{
    let events = Events::default();
    let server = RunningServer::start(Lifecycle {
        events: events.clone(),
        starts: Ok(()),
    })
    .await?;

    let response = reqwest::get(server.url("/")).await?;
    server.stop().await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(events.recorded(), ["startup", "request", "shutdown"]);
    Ok(())
}

#[tokio::test]
async fn releases_the_port_without_shutting_down_when_the_handler_fails_to_start() -> TestResult {
    let events = Events::default();
    let server = Server::bind("127.0.0.1:0").await?;
    let address = server.local_address()?;

    let outcome = server
        .serve(Lifecycle {
            events: events.clone(),
            starts: Err("no database"),
        })
        .await;

    let Err(error @ ServerError::Startup(_)) = outcome else {
        return Err(format!("unexpected outcome: {outcome:?}").into());
    };
    assert_eq!(error.to_string(), "the app failed to start");
    assert_eq!(
        error.source().map(ToString::to_string).as_deref(),
        Some("no database")
    );
    assert_eq!(events.recorded(), ["startup"]);
    Server::bind(address).await?;
    Ok(())
}

#[tokio::test]
async fn a_second_shutdown_request_skips_the_rest_of_the_shutdown() -> TestResult {
    let events = Events::default();
    let server = RunningServer::start(Lifecycle {
        events: events.clone(),
        starts: Ok(()),
    })
    .await?;
    let hanging = tokio::spawn(reqwest::get(server.url("/hang")));
    timeout(TEST_TIMEOUT, async {
        while !events.recorded().contains(&"request") {
            tokio::task::yield_now().await;
        }
    })
    .await?;

    server.shutdown.notify_one();
    // the port is free once serve has stopped accepting and waits on the hanging request
    timeout(TEST_TIMEOUT, async {
        while Server::bind(server.address).await.is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    server.shutdown.notify_one();

    timeout(TEST_TIMEOUT, server.task).await???;
    assert_eq!(events.recorded(), ["startup", "request"]);
    hanging.abort();
    Ok(())
}
