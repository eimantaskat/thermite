use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};

use http::{Method, StatusCode};
use serde_json::{Value, json};
use thermite_core::{
    Arguments, BodyDecoder, Request, Response, RouteDescription, RouteHandler, Router, Schema,
};
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::support::{RunningServer, TEST_TIMEOUT, TestResult, text_response};

// above the size the router reads away from the async worker
const LARGE: usize = 80_000;

fn integers(count: usize) -> String {
    format!("[{}]", vec!["1"; count].join(","))
}

fn counting_body() -> impl RouteHandler {
    |_request: Request, arguments: Arguments| async move {
        let count = arguments
            .body()
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        Response::new(count.to_string().into())
    }
}

async fn post(server: &RunningServer, body: String) -> reqwest::Result<reqwest::Response> {
    reqwest::Client::new()
        .post(server.url("/items"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
}

#[tokio::test]
async fn validates_a_large_body_like_a_small_one() -> TestResult {
    let mut router = Router::new();
    router.route(
        RouteDescription::new(Method::POST, "/items").body(Schema::array(Schema::integer())),
        counting_body(),
    )?;
    let server = RunningServer::start(router).await?;
    let mut invalid = vec!["1"; LARGE / 2];
    invalid[30_000] = "\"x\"";

    let small = post(&server, integers(3)).await?;
    let large = post(&server, integers(LARGE / 2)).await?;
    let rejected = post(&server, format!("[{}]", invalid.join(","))).await?;
    let broken = post(&server, format!("{} 2", integers(LARGE / 2))).await?;

    assert_eq!(small.text().await?, "3");
    assert_eq!(large.text().await?, (LARGE / 2).to_string());
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let rejected: Value = serde_json::from_str(&rejected.text().await?)?;
    assert_eq!(rejected["detail"][0]["loc"], json!(["body", 30_000]));
    assert_eq!(broken.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let broken: Value = serde_json::from_str(&broken.text().await?)?;
    assert_eq!(broken["detail"][0]["type"], "json_invalid");
    server.stop().await
}

// holds a large body until the test lets it go, and remembers whether it had to give up waiting
struct Holding {
    started: Arc<Notify>,
    release: Mutex<Receiver<()>>,
    gave_up: Arc<AtomicBool>,
}

impl BodyDecoder for Holding {
    fn decode(&self, _body: &[u8]) -> Option<Box<dyn Any + Send>> {
        self.started.notify_one();
        let released = self
            .release
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recv_timeout(TEST_TIMEOUT);
        self.gave_up.store(released.is_err(), Ordering::SeqCst);
        None
    }
}

// a current-thread runtime has one worker, which a body read on it would keep from every other request
#[tokio::test]
async fn answers_other_requests_while_a_large_body_is_read() -> TestResult {
    let started = Arc::new(Notify::new());
    let gave_up = Arc::new(AtomicBool::new(false));
    let (release, released) = mpsc::channel::<()>();
    let decoder = Holding {
        started: Arc::clone(&started),
        release: Mutex::new(released),
        gave_up: Arc::clone(&gave_up),
    };
    let mut router = Router::new();
    router.route(
        RouteDescription::new(Method::POST, "/items")
            .body_with_decoder(Schema::array(Schema::integer()), decoder),
        counting_body(),
    )?;
    router.route(
        RouteDescription::new(Method::GET, "/ping"),
        |_request: Request, _arguments: Arguments| async { text_response(StatusCode::OK, "pong") },
    )?;
    let server = RunningServer::start(router).await?;
    let url = server.url("/items");
    let large = tokio::spawn(async move {
        reqwest::Client::new()
            .post(url)
            .header("content-type", "application/json")
            .body(integers(LARGE / 2))
            .send()
            .await
    });
    timeout(TEST_TIMEOUT, started.notified()).await?;

    let ping = reqwest::get(server.url("/ping")).await?;
    release.send(())?;
    let large = timeout(TEST_TIMEOUT, large).await???;

    assert_eq!(ping.text().await?, "pong");
    assert_eq!(large.text().await?, (LARGE / 2).to_string());
    assert!(!gave_up.load(Ordering::SeqCst), "the body held up the ping");
    server.stop().await
}

struct Panicking;

impl BodyDecoder for Panicking {
    fn decode(&self, body: &[u8]) -> Option<Box<dyn Any + Send>> {
        assert!(body.is_empty(), "the decoder failed");
        None
    }
}

#[tokio::test]
async fn a_decoder_panicking_on_a_large_body_answers_500_like_on_a_small_one() -> TestResult {
    let mut router = Router::new();
    router.route(
        RouteDescription::new(Method::POST, "/items")
            .body_with_decoder(Schema::array(Schema::integer()), Panicking),
        counting_body(),
    )?;
    let server = RunningServer::start(router).await?;

    let small = post(&server, integers(3)).await?;
    let large = post(&server, integers(LARGE / 2)).await?;

    assert_eq!(small.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(large.status(), StatusCode::INTERNAL_SERVER_ERROR);
    server.stop().await
}
