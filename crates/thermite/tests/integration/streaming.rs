use std::error::Error;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use thermite::http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING};
use thermite::{
    App, BackgroundTasks, EventSourceResponse, ServerSentEvent, StatusCode, StreamingResponse,
    TestClient, TestRequestError,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::support::{RunningServer, TestResult};

// the chunks the test sends, so it decides when the stream produces each
#[derive(Clone)]
struct Chunks(Arc<Mutex<Option<UnboundedReceiver<&'static str>>>>);

impl Chunks {
    fn new() -> (UnboundedSender<&'static str>, Self) {
        let (sender, receiver) = unbounded_channel();
        (sender, Self(Arc::new(Mutex::new(Some(receiver)))))
    }

    fn stream(&self) -> impl futures_util::Stream<Item = &'static str> + Send + use<> {
        let receiver = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        stream::unfold(receiver, |receiver| async move {
            let mut receiver = receiver?;
            let chunk = receiver.recv().await?;
            Some((chunk, Some(receiver)))
        })
    }
}

fn streaming_app(chunks: Chunks) -> Result<App, Box<dyn Error>> {
    Ok(App::new().get("/report", move || {
        let chunks = chunks.stream();
        async move { StreamingResponse::new(chunks).media_type("text/csv") }
    })?)
}

#[tokio::test]
async fn reads_a_streamed_body_whole_with_its_chunked_framing() -> TestResult {
    let (sender, chunks) = Chunks::new();
    let client = TestClient::start(streaming_app(chunks)?).await?;
    sender.send("a,b\n")?;
    sender.send("1,2\n")?;
    drop(sender);

    let response = client.get("/report").send().await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "text/csv; charset=utf-8");
    assert_eq!(response.headers()[TRANSFER_ENCODING], "chunked");
    assert!(!response.headers().contains_key(CONTENT_LENGTH));
    assert_eq!(response.text()?, "a,b\n1,2\n");
    let response = response.into_response();
    assert_eq!(
        response.body().as_bytes().map(AsRef::as_ref),
        Some(&b"a,b\n1,2\n"[..])
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn reads_each_chunk_as_the_stream_produces_it() -> TestResult {
    let (sender, chunks) = Chunks::new();
    let client = TestClient::start(streaming_app(chunks)?).await?;

    let mut response = client.get("/report").stream().await?;
    sender.send("first")?;
    let first = response.chunk().await.transpose()?;
    sender.send("second")?;
    let second = response.chunk().await.transpose()?;
    drop(sender);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "text/csv; charset=utf-8");
    assert_eq!(first.as_deref(), Some(&b"first"[..]));
    assert_eq!(second.as_deref(), Some(&b"second"[..]));
    assert!(response.chunk().await.is_none());
    assert!(response.into_response().body().is_stream());
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn sends_each_chunk_over_the_wire_before_the_stream_ends() -> TestResult {
    let (sender, chunks) = Chunks::new();
    let server = RunningServer::start(streaming_app(chunks)?).await?;

    let mut response = reqwest::get(server.url("/report")).await?;
    sender.send("first")?;
    let first = response.chunk().await?;
    sender.send("second")?;
    let second = response.chunk().await?;
    drop(sender);
    let end = response.chunk().await?;

    assert_eq!(response.headers()[TRANSFER_ENCODING], "chunked");
    assert_eq!(first.as_deref(), Some(&b"first"[..]));
    assert_eq!(second.as_deref(), Some(&b"second"[..]));
    assert_eq!(end, None);
    server.stop().await
}

// after the head and the first chunk have reached the client, which an error at once could close the connection before
fn later<T>(item: T) -> impl futures_util::Stream<Item = T> {
    stream::once(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        item
    })
}

fn failing_app(log: &Log) -> Result<App, Box<dyn Error>> {
    let log = log.clone();
    Ok(App::new().get("/fails", move |tasks: BackgroundTasks| {
        let log = log.clone();
        async move {
            tasks.add(async move { log.record("task") });
            let failure = later(Err(io::Error::other("the database went away")));
            StreamingResponse::from_results(stream::iter([Ok("partial")]).chain(failure))
        }
    })?)
}

#[tokio::test]
async fn an_error_in_the_stream_cuts_the_response_short_and_skips_the_tasks() -> TestResult {
    let log = Log::default();
    let client = TestClient::start(failing_app(&log)?).await?;
    let server = RunningServer::start(failing_app(&log)?).await?;

    let whole = client.get("/fails").send().await;
    let mut streamed = client.get("/fails").stream().await?;
    let first = streamed.chunk().await.transpose()?;
    let failed = streamed.chunk().await;
    let mut wire = reqwest::get(server.url("/fails")).await?;
    let wire_first = wire.chunk().await?;
    let wire_failed = wire.chunk().await;

    let Err(TestRequestError::ResponseBody(error)) = whole else {
        return Err("the whole body was read".into());
    };
    let cause = error.get_ref().downcast_ref::<io::Error>();
    assert_eq!(
        cause.map(ToString::to_string).as_deref(),
        Some("the database went away")
    );
    assert_eq!(first.as_deref(), Some(&b"partial"[..]));
    assert!(matches!(failed, Some(Err(_))));
    assert!(streamed.chunk().await.is_none());
    assert_eq!(wire_first.as_deref(), Some(&b"partial"[..]));
    assert!(wire_failed.is_err());
    client.stop().await;
    server.stop().await?;
    assert_eq!(log.recorded(), Vec::<&str>::new());
    Ok(())
}

fn panicking_app() -> Result<App, Box<dyn Error>> {
    Ok(App::new().get("/panics", || async {
        let chunks = stream::iter(["partial"]).chain(later("never"));
        StreamingResponse::new(chunks.map(|chunk| {
            assert_ne!(chunk, "never", "the stream panicked");
            chunk
        }))
    })?)
}

#[tokio::test]
async fn a_panic_in_the_stream_is_caught_as_an_error_unless_the_test_reaches_it() -> TestResult {
    let caught = TestClient::start(panicking_app()?).await?.catch_panics();
    let reached = Arc::new(TestClient::start(panicking_app()?).await?);
    let server = RunningServer::start(panicking_app()?).await?;

    let whole = caught.get("/panics").send().await;
    let outcome = tokio::spawn({
        let reached = Arc::clone(&reached);
        async move { reached.get("/panics").send().await.map(|_| ()) }
    })
    .await;
    let mut wire = reqwest::get(server.url("/panics")).await?;
    let wire_first = wire.chunk().await?;
    let wire_failed = wire.chunk().await;

    assert!(matches!(whole, Err(TestRequestError::ResponseBody(_))));
    let Err(panic) = outcome else {
        return Err("the panic did not reach the test".into());
    };
    assert_eq!(
        panic
            .into_panic()
            .downcast_ref::<String>()
            .map(String::as_str),
        Some(
            "assertion `left != right` failed: the stream panicked\n  left: \"never\"\n right: \"never\""
        )
    );
    assert_eq!(wire_first.as_deref(), Some(&b"partial"[..]));
    assert!(wire_failed.is_err());
    caught.stop().await;
    server.stop().await
}

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<&'static str>>>);

impl Log {
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

#[tokio::test]
async fn background_tasks_run_once_the_stream_has_ended() -> TestResult {
    let (sender, chunks) = Chunks::new();
    let log = Log::default();
    let app = App::new().get("/report", {
        let log = log.clone();
        move |tasks: BackgroundTasks| {
            let chunks = chunks.stream();
            let log = log.clone();
            async move {
                tasks.add(async move { log.record("task") });
                StreamingResponse::new(chunks)
            }
        }
    })?;
    let client = TestClient::start(app).await?;

    let mut response = client.get("/report").stream().await?;
    sender.send("chunk")?;
    let chunk = response.chunk().await.transpose()?;
    let while_streaming = log.recorded();
    drop(sender);
    let end = response.chunk().await;

    assert_eq!(chunk.as_deref(), Some(&b"chunk"[..]));
    assert_eq!(while_streaming, Vec::<&str>::new());
    assert!(end.is_none());
    assert_eq!(log.recorded(), ["task"]);
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn a_head_request_is_framed_as_the_server_frames_it() -> TestResult {
    let app = || {
        App::new().head("/report", || async {
            StreamingResponse::new(stream::iter(["never sent"]))
        })
    };
    let client = TestClient::start(app()?).await?;
    let server = RunningServer::start(app()?).await?;

    let in_process = client.head("/report").send().await?;
    let wire = reqwest::Client::new()
        .head(server.url("/report"))
        .send()
        .await?;

    let framing = |headers: &thermite::http::HeaderMap| {
        (
            headers.get(TRANSFER_ENCODING).cloned(),
            headers.get(CONTENT_LENGTH).cloned(),
        )
    };
    assert_eq!(framing(in_process.headers()), framing(wire.headers()));
    assert!(in_process.body().is_empty());
    client.stop().await;
    server.stop().await
}

fn events_app() -> Result<App, Box<dyn Error>> {
    Ok(App::new().get("/events", || async {
        let events = stream::iter([json!({"n": 1}), json!({"n": 2})])
            .map(|item| ServerSentEvent::new().data(&item)?.event("count"));
        EventSourceResponse::from_results(events)
    })?)
}

#[tokio::test]
async fn sends_server_sent_events_as_fastapi_writes_them() -> TestResult {
    let client = TestClient::start(events_app()?).await?;
    let server = RunningServer::start(events_app()?).await?;

    let in_process = client.get("/events").send().await?;
    let wire = reqwest::get(server.url("/events")).await?;
    let wire_type = wire.headers()[CONTENT_TYPE].clone();
    let wire_body = wire.text().await?;

    let expected = "event: count\ndata: {\"n\":1}\n\nevent: count\ndata: {\"n\":2}\n\n";
    assert_eq!(in_process.text()?, expected);
    assert_eq!(
        in_process.headers()[CONTENT_TYPE],
        "text/event-stream; charset=utf-8"
    );
    assert_eq!(in_process.headers()[CACHE_CONTROL], "no-cache");
    assert_eq!(wire_type, "text/event-stream; charset=utf-8");
    assert_eq!(wire_body, expected);
    client.stop().await;
    server.stop().await
}

#[tokio::test]
async fn an_endless_event_stream_is_read_event_by_event() -> TestResult {
    let app = App::new().get("/ticks", || async {
        let ticks = stream::iter(0..).then(|tick| async move {
            tokio::time::sleep(Duration::from_millis(1)).await;
            ServerSentEvent::new().raw_data(tick.to_string())
        });
        EventSourceResponse::new(ticks)
    })?;
    let client = TestClient::start(app).await?;

    let mut response = client.get("/ticks").stream().await?;
    let first = response.chunk().await.transpose()?;
    let second = response.chunk().await.transpose()?;
    drop(response);

    assert_eq!(first.as_deref(), Some(&b"data: 0\n\n"[..]));
    assert_eq!(second.as_deref(), Some(&b"data: 1\n\n"[..]));
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn documents_a_streamed_route_as_a_successful_response() -> TestResult {
    let client = TestClient::start(events_app()?).await?;

    let document: Value = client.get("/openapi.json").send().await?.json()?;

    assert_eq!(
        document["paths"]["/events"]["get"]["responses"]["200"]["description"],
        "Successful Response"
    );
    client.stop().await;
    Ok(())
}
