use std::error::Error;
use std::future::{Future, pending, ready};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::StatusCode;
use thermite_core::{
    BackgroundTasks, InProcessServer, Request, RequestHandler, Response, attach_background_tasks,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::support::{Events, RunningServer, TEST_TIMEOUT, TestResult, text_response};

// a task that panics is what some of these tests are about
#[allow(clippy::panic)]
fn fail() {
    panic!("task failed");
}

// each path adds its own tasks to the response
struct AddingTasks {
    events: Events,
    release: Arc<Notify>,
}

impl AddingTasks {
    fn new(events: &Events) -> Self {
        Self {
            events: events.clone(),
            release: Arc::new(Notify::new()),
        }
    }
}

impl RequestHandler for AddingTasks {
    fn call(self: &Arc<Self>, request: Request) -> impl Future<Output = Response> + Send + 'static {
        self.events.record("request");
        let tasks = BackgroundTasks::new();
        let (first, second) = (self.events.clone(), self.events.clone());
        match request.uri().path() {
            "/waits" => {
                let release = Arc::clone(&self.release);
                tasks.add(async move {
                    release.notified().await;
                    first.record("first");
                });
                tasks.add(async move { second.record("second") });
            }
            "/panics" => {
                tasks.add(async move { first.record("before the panic") });
                tasks.add(async { fail() });
                tasks.add(async move { second.record("after the panic") });
            }
            "/slow" => tasks.add(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                first.record("slow");
            }),
            "/endless" => tasks.add(pending()),
            "/quick" => tasks.add(async move { first.record("quick") }),
            _ => {}
        }
        let mut response = text_response(StatusCode::OK, "sent");
        attach_background_tasks(&mut response, tasks);
        ready(response)
    }

    fn shutdown(&self) -> impl Future<Output = ()> + Send {
        self.events.record("shutdown");
        ready(())
    }
}

fn get(path: &str) -> Result<Request, http::Error> {
    http::Request::builder().uri(path).body(Bytes::new())
}

// both go on one connection, which the second closes once it is answered
async fn send_two(
    address: SocketAddr,
    first: &str,
    second: &str,
) -> Result<String, Box<dyn Error>> {
    let mut stream = TcpStream::connect(address).await?;
    let requests = format!(
        "GET {first} HTTP/1.1\r\nhost: localhost\r\n\r\n\
         GET {second} HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n"
    );
    stream.write_all(requests.as_bytes()).await?;
    let mut responses = String::new();
    timeout(TEST_TIMEOUT, stream.read_to_string(&mut responses)).await??;
    Ok(responses)
}

// as in uvicorn, where each request and its tasks are an asyncio task of their own
#[tokio::test]
async fn runs_the_tasks_of_the_next_response_while_the_earlier_ones_still_run() -> TestResult {
    let events = Events::default();
    let handler = AddingTasks::new(&events);
    let release = Arc::clone(&handler.release);
    let server = RunningServer::start(handler).await?;

    let responses = send_two(server.address, "/waits", "/quick").await?;
    timeout(TEST_TIMEOUT, async {
        while !events.recorded().contains(&"quick") {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let before_release = events.recorded();
    release.notify_one();
    server.stop().await?;

    assert_eq!(responses.matches("HTTP/1.1 200 OK").count(), 2);
    assert_eq!(before_release, ["request", "request", "quick"]);
    assert_eq!(
        events.recorded(),
        ["request", "request", "quick", "first", "second", "shutdown"]
    );
    Ok(())
}

#[tokio::test]
async fn a_panicking_task_skips_the_tasks_after_it_but_not_the_next_response() -> TestResult {
    let events = Events::default();
    let server = RunningServer::start(AddingTasks::new(&events)).await?;

    let responses = send_two(server.address, "/panics", "/panics").await?;
    server.stop().await?;

    let recorded = events.recorded();
    let count = |event| {
        recorded
            .iter()
            .filter(|&&recorded| recorded == event)
            .count()
    };
    assert_eq!(responses.matches("HTTP/1.1 200 OK").count(), 2);
    assert_eq!(count("before the panic"), 2);
    assert_eq!(count("after the panic"), 0);
    Ok(())
}

// hyper sends the response to a HEAD request without reading its body
#[tokio::test]
async fn waits_for_the_tasks_before_shutting_the_handler_down() -> TestResult {
    let events = Events::default();
    let server = RunningServer::start(AddingTasks::new(&events)).await?;

    let response = reqwest::Client::new()
        .head(server.url("/slow"))
        .send()
        .await?;
    server.stop().await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(events.recorded(), ["request", "slow", "shutdown"]);
    Ok(())
}

#[tokio::test]
async fn drops_the_tasks_still_running_when_the_shutdown_timeout_passes() -> TestResult {
    let events = Events::default();
    let server = RunningServer::start_configured(
        |server| server.with_shutdown_timeout(Duration::from_millis(100)),
        AddingTasks::new(&events),
    )
    .await?;

    let response = reqwest::get(server.url("/endless")).await?;
    server.stop().await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(events.recorded(), ["request", "shutdown"]);
    Ok(())
}

#[tokio::test]
async fn send_in_process_returns_once_the_tasks_have_run() -> TestResult {
    let events = Events::default();
    let server = InProcessServer::start(AddingTasks::new(&events)).await?;

    let response = server.send(get("/slow")?).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(events.recorded(), ["request", "slow"]);
    Ok(())
}

#[tokio::test]
async fn a_panicking_task_reaches_the_caller_in_process_unless_caught() -> TestResult {
    let events = Events::default();
    let server = Arc::new(InProcessServer::start(AddingTasks::new(&events)).await?);
    let caught = InProcessServer::start(AddingTasks::new(&events))
        .await?
        .catch_panics();

    let outcome = tokio::spawn({
        let server = Arc::clone(&server);
        let request = get("/panics")?;
        async move { server.send(request).await }
    })
    .await;
    let response = timeout(TEST_TIMEOUT, caught.send(get("/panics")?)).await?;

    let Err(error) = outcome else {
        return Err("the panic did not reach the caller".into());
    };
    assert_eq!(
        error.into_panic().downcast_ref::<&str>(),
        Some(&"task failed")
    );
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.body().as_ref(), b"sent");
    assert_eq!(
        events.recorded(),
        ["request", "before the panic", "request", "before the panic"]
    );
    Ok(())
}
