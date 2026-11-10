use std::convert::Infallible;
use std::error::Error;
use std::future::{Future, ready};
use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream;
use http::header::{DATE, TRANSFER_ENCODING};
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use thermite_core::{Body, InProcessServer, Request, RequestHandler, Response, ServerError};

use crate::support::{Events, RunningServer, TestResult, text_response};

// what the wire shows of a response, apart from the date hyper adds
#[derive(Debug, PartialEq)]
struct Seen {
    status: StatusCode,
    headers: Vec<(String, HeaderValue)>,
    body: Bytes,
}

impl Seen {
    fn new(status: StatusCode, headers: &HeaderMap, body: Bytes) -> Self {
        let mut headers: Vec<_> = headers
            .iter()
            .filter(|(name, _)| **name != DATE)
            .map(|(name, value)| (name.as_str().to_owned(), value.clone()))
            .collect();
        headers.sort_by(|left, right| left.0.cmp(&right.0));
        Self {
            status,
            headers,
            body,
        }
    }
}

// a handler that panics is part of what the server must answer like
#[allow(clippy::panic)]
fn respond(path: &str) -> Response {
    match path {
        "/created" => {
            let mut response = text_response(StatusCode::CREATED, "created");
            response
                .headers_mut()
                .insert("x-thermite", HeaderValue::from_static("hot"));
            response
        }
        "/no-content" => text_response(StatusCode::NO_CONTENT, "dropped"),
        "/not-modified" => text_response(StatusCode::NOT_MODIFIED, "dropped"),
        "/panic" => panic!("handler failed"),
        "/streamed" => streamed(StatusCode::OK),
        "/streamed-no-content" => streamed(StatusCode::NO_CONTENT),
        "/streamed-not-modified" => streamed(StatusCode::NOT_MODIFIED),
        "/chunked-whole" => chunked(text_response(StatusCode::OK, "whole")),
        "/chunked-stream" => chunked(streamed(StatusCode::OK)),
        _ => text_response(StatusCode::OK, ""),
    }
}

fn streamed(status: StatusCode) -> Response {
    let chunks = stream::iter(["one", "two"].map(Ok::<_, Infallible>));
    let mut response = Response::new(Body::from_stream(chunks));
    *response.status_mut() = status;
    response
}

fn chunked(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
    response
}

fn answering_by_path() -> impl RequestHandler {
    |request: Request| ready(respond(request.uri().path()))
}

async fn over_the_wire(
    requests: &[(Method, &str, &'static str)],
) -> Result<Vec<Seen>, Box<dyn Error>> {
    let server =
        RunningServer::start_configured(|server| server.with_body_limit(4), answering_by_path())
            .await?;
    let client = reqwest::Client::new();
    let mut seen = Vec::new();
    for (method, path, body) in requests {
        let response = client
            .request(method.clone(), server.url(path))
            .body(*body)
            .send()
            .await?;
        let status = response.status();
        let headers = response.headers().clone();
        seen.push(Seen::new(status, &headers, response.bytes().await?));
    }
    server.stop().await?;
    Ok(seen)
}

async fn in_process(
    requests: &[(Method, &str, &'static str)],
) -> Result<Vec<Seen>, Box<dyn Error>> {
    let server = InProcessServer::start(answering_by_path())
        .await?
        .with_body_limit(4)
        .catch_panics();
    let mut seen = Vec::new();
    for (method, path, body) in requests {
        let request = http::Request::builder()
            .method(method.clone())
            .uri(*path)
            .body(Bytes::from_static(body.as_bytes()))?;
        let (parts, body) = server.send(request).await?.into_parts();
        seen.push(Seen::new(parts.status, &parts.headers, body));
    }
    server.stop().await;
    Ok(seen)
}

#[tokio::test]
async fn answers_in_process_as_the_server_does_over_the_wire() -> TestResult {
    let requests = [
        (Method::GET, "/created", ""),
        (Method::HEAD, "/created", ""),
        (Method::GET, "/empty", ""),
        (Method::GET, "/no-content", ""),
        (Method::GET, "/not-modified", ""),
        (Method::POST, "/created", "abcde"),
        (Method::GET, "/panic", ""),
        (Method::GET, "/streamed", ""),
        (Method::HEAD, "/streamed", ""),
        (Method::GET, "/streamed-no-content", ""),
        (Method::GET, "/streamed-not-modified", ""),
        (Method::GET, "/chunked-whole", ""),
        (Method::HEAD, "/chunked-whole", ""),
        (Method::GET, "/chunked-stream", ""),
        (Method::HEAD, "/chunked-stream", ""),
    ];

    assert_eq!(
        in_process(&requests).await?,
        over_the_wire(&requests).await?
    );
    Ok(())
}

#[tokio::test]
async fn a_panic_reaches_the_caller_unless_caught() -> TestResult {
    let server = Arc::new(InProcessServer::start(answering_by_path()).await?);
    let request = http::Request::builder().uri("/panic").body(Bytes::new())?;

    let outcome = tokio::spawn({
        let server = Arc::clone(&server);
        async move { server.send(request).await }
    })
    .await;

    let Err(error) = outcome else {
        return Err("the panic did not reach the caller".into());
    };
    assert_eq!(
        error.into_panic().downcast_ref::<&str>(),
        Some(&"handler failed")
    );
    Ok(())
}

struct Lifecycle {
    events: Events,
    starts: Result<(), &'static str>,
}

impl RequestHandler for Lifecycle {
    fn call(self: &Arc<Self>, _: Request) -> impl Future<Output = Response> + Send + 'static {
        self.events.record("request");
        ready(text_response(StatusCode::OK, ""))
    }

    fn startup(&self) -> impl Future<Output = Result<(), Box<dyn Error + Send + Sync>>> + Send {
        self.events.record("startup");
        ready(self.starts.map_err(Into::into))
    }

    fn shutdown(&self) -> impl Future<Output = ()> + Send {
        self.events.record("shutdown");
        ready(())
    }
}

#[tokio::test]
async fn starts_the_handler_when_started_and_shuts_it_down_when_stopped() -> TestResult {
    let events = Events::default();
    let server = InProcessServer::start(Lifecycle {
        events: events.clone(),
        starts: Ok(()),
    })
    .await?;

    server.send(Request::default()).await?;
    server.stop().await;

    assert_eq!(events.recorded(), ["startup", "request", "shutdown"]);
    Ok(())
}

#[tokio::test]
async fn reports_a_handler_that_fails_to_start() -> TestResult {
    let events = Events::default();

    let outcome = InProcessServer::start(Lifecycle {
        events: events.clone(),
        starts: Err("no database"),
    })
    .await;

    let Some(error) = outcome.err() else {
        return Err("the handler started".into());
    };
    assert!(matches!(error, ServerError::Startup(_)));
    assert_eq!(
        error.source().map(ToString::to_string).as_deref(),
        Some("no database")
    );
    assert_eq!(events.recorded(), ["startup"]);
    Ok(())
}
