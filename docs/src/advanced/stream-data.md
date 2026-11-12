# Stream data

A streamed response sends its body piece by piece as it is made, instead of building it whole in memory first.
A large export, a file read in parts or the output of a long job are the usual examples.
This is FastAPI's `StreamingResponse`, and events for a browser are on the next page, [Server-sent events](server-sent-events.md).

```rust
{{#include ../../../docs_src/src/bin/stream_data.rs:imports}}
```

## A stream of chunks

```rust
{{#include ../../../docs_src/src/bin/stream_data.rs:report}}
```

FastAPI streams an `async def` that `yield`s.
Rust has no async generators yet, so the body is a `Stream`, the async counterpart of an iterator: something that gives one item after another, waiting between them when it needs to.
`StreamingResponse::new` takes any stream whose items turn into bytes, such as `String`, `&'static str`, `Vec<u8>` or `Bytes`.

The [`futures-util`](https://crates.io/crates/futures-util) crate builds them, the one crate this page adds; `tokio::time::sleep` also needs tokio's `time` feature:

- `stream::iter(1..=3)` turns an iterator into a stream, here the numbers 1 to 3;
- `.then(|row| async move { ... })` runs an async block for each item and gives what it returns, one after another;
- `.map(...)` changes each item without waiting, and `stream::unfold` builds a stream from a state and a step, the closest to a generator's loop.

`then` and `map` are methods of the `StreamExt` trait, which is why the imports name it: a trait's methods can be called only where the trait is imported.

`media_type("text/csv")` sets the `Content-Type`, and a `text/` type gets `; charset=utf-8` as in Starlette, so `/report` answers:

```text
content-type: text/csv; charset=utf-8
transfer-encoding: chunked

1,1
2,4
3,9
```

The body has no `Content-Length`, since its size is known only at the end, so it is sent in HTTP's chunked framing and each row reaches the client as soon as it is made.
Without `media_type` the response has no `Content-Type`, as in Starlette.

## What the stream can hold

The stream has to be `Send + 'static`: it runs after the handler has returned, and on whichever thread the runtime picks.
`'static` means the stream owns what it uses rather than borrowing it from the handler, so values go in with `move`, as for [background tasks](../tutorial/background-tasks.md).
`Send` means it can move between threads, which rules out holding a `std::sync::MutexGuard` or an `Rc` across an `.await` inside it.

A blocking read, such as `std::fs::File`, would hold up the thread while it waits.
`tokio::fs` reads files without blocking, and the [`tokio-util`](https://crates.io/crates/tokio-util) crate's `ReaderStream` turns any async reader into a stream of chunks.

## Errors in the stream

```rust
{{#include ../../../docs_src/src/bin/stream_data.rs:export}}
```

`StreamingResponse::from_results` takes a stream of `Result`s.
The status and the first rows are sent by the time the third row fails, so the error can't become a 500.
It ends the response early instead and closes the connection, as an exception in a FastAPI generator does once the response has started, and the server logs it.

The client sees `row 1` and `row 2`, and then a body cut short, which an HTTP client reports as an error rather than as a complete response.
Background tasks of a response whose stream fails are skipped, as in Starlette; otherwise they run once the whole body is sent.

```rust
{{#include ../../../docs_src/src/bin/stream_data.rs:app}}
```

`cargo run -p docs_src --bin stream_data` runs this page's app, and `main` is the same as in [First steps](../tutorial/first-steps.md).
`curl -N http://127.0.0.1:8000/report` shows the rows arriving one by one.

## In the docs

A streamed response is listed as a 200 without content, since the stream's type says nothing of what its bytes mean.
FastAPI lists it as a JSON 200, unless `response_class` says otherwise.

## What is missing

FastAPI also streams a handler that yields models as JSON lines, here written as a `StreamingResponse` whose items are made with `serde_json::to_string`.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/stream_data.rs:test}}
```

`send()` waits for the whole body, as for any response, and fails with `TestRequestError::ResponseBody` when the stream does.
`stream()` gives a `TestStreamingResponse` instead, with the status and headers at once and the body through `chunk().await`, one chunk at a time, so even an endless stream can be tested.
`chunk()` returns `None` at the end, and `Some(Err(...))` when the stream fails.

`.transpose()` turns the `Option<Result<..>>` that `chunk()` returns into a `Result<Option<..>>`, so `?` can fail the test on an error and leave the `Option` to compare.
