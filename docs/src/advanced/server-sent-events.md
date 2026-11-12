# Server-sent events

Server-sent events are a [stream](stream-data.md) a browser reads with `EventSource`, getting one event at a time over a single response that stays open.
They suit updates that go only from the server to the client, such as progress or notifications; for messages both ways there are [WebSockets](websockets.md).
This is FastAPI's `EventSourceResponse` and `ServerSentEvent`, added in FastAPI 0.135.

```rust
{{#include ../../../docs_src/src/bin/server_sent_events.rs:imports}}

{{#include ../../../docs_src/src/bin/server_sent_events.rs:items}}
```

## Sending events

```rust
{{#include ../../../docs_src/src/bin/server_sent_events.rs:stream}}
```

A `ServerSentEvent` is built field by field, under the names FastAPI's has:

| Method | Field | Fails |
| --- | --- | --- |
| `.data(&value)` | `data`, the value as JSON | when the value doesn't serialize |
| `.raw_data(text)` | `data`, the text as it is | never |
| `.event(name)` | `event`, which a browser listens for with `addEventListener(name, ...)` | on a line break |
| `.id(id)` | `id`, which the browser sends back when it reconnects | on a line break or a null |
| `.retry(duration)` | `retry`, how long the browser waits before reconnecting | never |
| `.comment(text)` | a comment line, which the browser ignores | never |

The ones that can fail return a `Result`, since a line break in an event's name would end the field early and let the rest be read as another field.
`item_event` chains them with `?`, so the first failure is its `Err`.

`EventSourceResponse::from_results` takes a stream of `Result<ServerSentEvent, _>`, and ends the response at the first error, as `StreamingResponse::from_results` does.
`EventSourceResponse::new` takes a stream of events for when nothing can fail.
Here `stream::iter` turns the list into a stream, and `.zip(0..)` pairs each item with its index, from 0 up, for the event's id.

`/items/stream` answers:

```text
content-type: text/event-stream; charset=utf-8
cache-control: no-cache
x-accel-buffering: no

event: item
data: {"name":"Plumbus","price":32.99}
id: 0

event: item
data: {"name":"Portal Gun","price":999.99}
id: 1

event: item
data: {"name":"Meeseeks Box","price":49.99}
id: 2

```

Each event is its fields, one per line, and an empty line.
`Cache-Control: no-cache` keeps a browser from caching the stream, and `X-Accel-Buffering: no` tells nginx not to hold the events back in its buffer.

When no event has been sent for 15 seconds, the response sends a `: ping` comment, as FastAPI does, so a proxy doesn't close a stream that looks idle.

## Resuming after a reconnect

```rust
{{#include ../../../docs_src/src/bin/server_sent_events.rs:resume}}
```

A browser whose connection drops reconnects on its own and sends the id of the last event it got in the `Last-Event-ID` header.
The handler reads it as a [header](../tutorial/header-cookie-params.md), `last_event_id` standing for `last-event-id`, and skips the events up to it.
With `Last-Event-ID: 1`, `/items/resumable` sends only the Meeseeks Box.

The iterator becomes the response's stream, which outlives the handler, so the `filter` closure takes `start` with `move` rather than borrowing it.
`saturating_add(1)` stops at the largest `i64` instead of overflowing, which would panic in a debug build.

```rust
{{#include ../../../docs_src/src/bin/server_sent_events.rs:app}}
```

`cargo run -p docs_src --bin server_sent_events` runs this page's app, and `main` is the same as in [First steps](../tutorial/first-steps.md).
`curl -N http://127.0.0.1:8000/items/stream` shows the events.

## In the docs

An event stream is listed as a 200 without content, where FastAPI lists `text/event-stream` with a schema for the events.

## What is missing

FastAPI also streams a handler that yields models with `response_class=EventSourceResponse`, writing each as an event's data; here each one is wrapped in a `ServerSentEvent` by hand.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/server_sent_events.rs:test}}
```

These streams end, so `send()` reads them whole.
An endless one, such as a feed of updates, is read event by event with `stream()` and `chunk()`, as on [Stream data](stream-data.md#testing-it).
