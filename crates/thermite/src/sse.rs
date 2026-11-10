use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use http::header::{CACHE_CONTROL, CONTENT_TYPE};
use http::{HeaderName, HeaderValue, StatusCode};
use serde::Serialize;
use thermite_core::{Body, BoxError, Response, ResponseDescription};
use tokio::time::{Instant, Sleep, sleep};

use crate::response::{IntoResponse, SUCCESSFUL_RESPONSE};

// FastAPI's, sent after this long without an event, so proxies keep an idle stream open
const PING_INTERVAL: Duration = Duration::from_secs(15);
const PING: &[u8] = b": ping\n\n";

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EventError {
    #[error("the event's data does not serialize to JSON")]
    Data(#[from] serde_json::Error),
    #[error("an event's name must be a single line")]
    MultilineEvent,
    #[error("an event's id must be a single line without a null character")]
    InvalidId,
}

// FastAPI's ServerSentEvent: one event of an EventSourceResponse
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerSentEvent {
    comment: Option<String>,
    event: Option<String>,
    data: Option<String>,
    id: Option<String>,
    retry: Option<Duration>,
}

impl ServerSentEvent {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // as JSON, as FastAPI writes `data`
    pub fn data(self, data: &impl Serialize) -> Result<Self, EventError> {
        Ok(self.raw_data(serde_json::to_string(data)?))
    }

    // as it is, as FastAPI writes `raw_data`
    #[must_use]
    pub fn raw_data(mut self, data: impl Into<String>) -> Self {
        self.data = Some(data.into());
        self
    }

    pub fn event(mut self, event: impl Into<String>) -> Result<Self, EventError> {
        let event = event.into();
        if event.contains(['\r', '\n']) {
            return Err(EventError::MultilineEvent);
        }
        self.event = Some(event);
        Ok(self)
    }

    pub fn id(mut self, id: impl Into<String>) -> Result<Self, EventError> {
        let id = id.into();
        if id.contains(['\r', '\n', '\0']) {
            return Err(EventError::InvalidId);
        }
        self.id = Some(id);
        Ok(self)
    }

    // how long the browser waits before reconnecting, sent in whole milliseconds
    #[must_use]
    pub fn retry(mut self, retry: Duration) -> Self {
        self.retry = Some(retry);
        self
    }

    #[must_use]
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    // in FastAPI's order: comment, event, data, id, retry, each line of a field under its own prefix
    fn encode(&self) -> Bytes {
        let mut encoded = String::new();
        let mut write = |prefix: &str, value: &str| {
            for line in value
                .split("\r\n")
                .flat_map(|line| line.split(['\r', '\n']))
            {
                encoded.push_str(prefix);
                encoded.push_str(line);
                encoded.push('\n');
            }
        };
        if let Some(comment) = &self.comment {
            write(": ", comment);
        }
        if let Some(event) = &self.event {
            write("event: ", event);
        }
        if let Some(data) = &self.data {
            write("data: ", data);
        }
        if let Some(id) = &self.id {
            write("id: ", id);
        }
        if let Some(retry) = self.retry {
            write("retry: ", &retry.as_millis().to_string());
        }
        encoded.push('\n');
        Bytes::from(encoded)
    }
}

// FastAPI's EventSourceResponse: a `text/event-stream` of events, pinged while idle
#[derive(Debug)]
pub struct EventSourceResponse {
    body: Body,
}

impl EventSourceResponse {
    pub fn new<Events>(events: Events) -> Self
    where
        Events: Stream<Item = ServerSentEvent> + Send + 'static,
    {
        Self::from_results(events.map(Ok::<_, Infallible>))
    }

    // an error ends the stream early, closing the connection
    pub fn from_results<Events, Error>(events: Events) -> Self
    where
        Events: Stream<Item = Result<ServerSentEvent, Error>> + Send + 'static,
        Error: Into<BoxError>,
    {
        let encoded = events.map(|event| event.map(|event| event.encode()));
        Self {
            body: Body::from_stream(Pinging::new(encoded)),
        }
    }
}

impl IntoResponse for EventSourceResponse {
    fn describe() -> Vec<ResponseDescription> {
        vec![ResponseDescription::new(
            StatusCode::OK,
            SUCCESSFUL_RESPONSE,
        )]
    }

    fn into_response(self) -> Response {
        let mut response = Response::new(self.body);
        let headers = response.headers_mut();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        // tells nginx not to hold the events back in its buffer
        headers.insert(
            HeaderName::from_static("x-accel-buffering"),
            HeaderValue::from_static("no"),
        );
        response
    }
}

// sends a ping comment whenever the events have been quiet for the ping interval
struct Pinging<Events> {
    events: Pin<Box<Events>>,
    // made on the first poll, so the clock starts with the stream and a response is built even off the runtime
    quiet: Option<Pin<Box<Sleep>>>,
}

impl<Events> Pinging<Events> {
    fn new(events: Events) -> Self {
        Self {
            events: Box::pin(events),
            quiet: None,
        }
    }
}

impl<Events, Error> Stream for Pinging<Events>
where
    Events: Stream<Item = Result<Bytes, Error>>,
{
    type Item = Result<Bytes, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let quiet = this
            .quiet
            .get_or_insert_with(|| Box::pin(sleep(PING_INTERVAL)));
        if let Poll::Ready(event) = this.events.as_mut().poll_next(cx) {
            quiet.as_mut().reset(Instant::now() + PING_INTERVAL);
            return Poll::Ready(event);
        }
        if quiet.as_mut().poll(cx).is_ready() {
            quiet.as_mut().reset(Instant::now() + PING_INTERVAL);
            return Poll::Ready(Some(Ok(Bytes::from_static(PING))));
        }
        Poll::Pending
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use futures_util::stream;
    use serde_json::json;

    use super::*;

    #[test]
    fn writes_each_field_in_fastapi_order_and_each_line_under_its_prefix() {
        let event = ServerSentEvent::new()
            .retry(Duration::from_millis(1500))
            .id("7")
            .unwrap()
            .raw_data("one\r\ntwo\rthree\nfour")
            .event("update")
            .unwrap()
            .comment("note");

        assert_eq!(
            event.encode(),
            ": note\nevent: update\ndata: one\ndata: two\ndata: three\ndata: four\nid: 7\nretry: 1500\n\n"
        );
        assert_eq!(ServerSentEvent::new().encode(), "\n");
    }

    #[test]
    fn writes_data_as_json() {
        let event = ServerSentEvent::new()
            .data(&json!({"name": "a\nb"}))
            .unwrap();

        assert_eq!(event.encode(), "data: {\"name\":\"a\\nb\"}\n\n");
    }

    #[test]
    fn refuses_what_would_break_the_event_apart() {
        assert!(matches!(
            ServerSentEvent::new().event("a\nb"),
            Err(EventError::MultilineEvent)
        ));
        for id in ["a\rb", "a\0b"] {
            assert!(matches!(
                ServerSentEvent::new().id(id),
                Err(EventError::InvalidId)
            ));
        }
        let unserializable = std::collections::BTreeMap::from([((1, 2), 3)]);
        assert!(matches!(
            ServerSentEvent::new().data(&unserializable),
            Err(EventError::Data(_))
        ));
    }

    #[tokio::test]
    async fn answers_an_event_stream_that_proxies_do_not_buffer() {
        let events = stream::iter([ServerSentEvent::new().raw_data("hi")]);

        let response = EventSourceResponse::new(events).into_response();

        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "text/event-stream; charset=utf-8"
        );
        assert_eq!(response.headers()[CACHE_CONTROL], "no-cache");
        assert_eq!(response.headers()["x-accel-buffering"], "no");
        assert_eq!(
            response.into_body().into_bytes().await.unwrap(),
            "data: hi\n\n"
        );
        assert_eq!(
            EventSourceResponse::describe(),
            [ResponseDescription::new(
                StatusCode::OK,
                SUCCESSFUL_RESPONSE
            )]
        );
    }

    #[test]
    fn an_event_stream_is_built_outside_a_runtime() {
        let response = EventSourceResponse::new(stream::empty()).into_response();

        assert!(response.body().is_stream());
    }

    #[tokio::test(start_paused = true)]
    async fn sends_no_ping_once_the_events_have_ended() {
        let events = stream::once(async {
            tokio::time::sleep(Duration::from_secs(20)).await;
            ServerSentEvent::new().raw_data("last")
        });
        let mut body = EventSourceResponse::new(events).into_response().into_body();

        let mut chunks = Vec::new();
        while let Some(chunk) = body.chunk().await {
            chunks.push(chunk.unwrap());
        }

        assert_eq!(
            chunks,
            [
                Bytes::from_static(PING),
                Bytes::from(
                    "data: last

"
                )
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn pings_only_after_the_events_have_been_quiet_for_the_interval() {
        let events = stream::once(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            ServerSentEvent::new().raw_data("late")
        })
        .chain(stream::pending());
        let mut body = EventSourceResponse::new(events).into_response().into_body();

        let started = Instant::now();
        let first = body.chunk().await.unwrap().unwrap();
        let after_first = started.elapsed();
        let second = body.chunk().await.unwrap().unwrap();

        assert_eq!(first, "data: late\n\n");
        assert_eq!(after_first, Duration::from_secs(10));
        assert_eq!(second, PING);
        assert_eq!(started.elapsed(), Duration::from_secs(25));
    }
}
