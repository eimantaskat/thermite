# Testing

`TestClient` sends requests to an app without opening a port, as FastAPI's `TestClient` does.
Every page of this tutorial ends with a test written this way.
This page's app reads an item and creates one, both behind a token header:

```rust
{{#include ../../../docs_src/src/bin/testing.rs:imports}}

{{#include ../../../docs_src/src/bin/testing.rs:app}}
```

`check_token(&header)?` returns the `HttpError` from the handler when the token is wrong, and goes on otherwise.

`cargo run -p docs_src --bin testing` runs this page's app, and `main` is the same as in [First steps](first-steps.md).

## The tests

```rust
{{#include ../../../docs_src/src/bin/testing.rs:test}}
```

`cargo test` runs them, and `cargo test refuses` runs only the tests whose names contain `refuses`.

## Step by step

### Where the tests live

`#[cfg(test)]` compiles the module only for `cargo test`, so the tests aren't part of the program you ship.
`use super::*` brings in everything from the file around the module, private items included, so the tests can call `app()`.

Tests that use only the crate's public items go in a `tests/` directory instead, one file per group, like FastAPI's `test_main.py`.
Those can't reach the items of a binary, so the app is then built in `src/lib.rs`, with a `pub fn app()`, and `src/main.rs` only serves it.

### Starting the client

`#[tokio::test]` runs the test on a tokio runtime, as `#[tokio::main]` runs `main`.
`TestClient::start` starts the app, running its startup as the server would, and fails if the startup does, such as when a route needs state the app was never given.
`client.stop()` stops it again, like leaving FastAPI's `with TestClient(app) as client:` block.

Each test returns a `Result`, so `?` can be used on anything that may fail, and an `Err` fails the test with its message.

### Requests

`client.get(path)` and `client.post(path)`, and the other methods, start a request.
It can be given:

- `.header(name, value)` for a header;
- `.json(&value)` for a JSON body made from any `Serialize` value, here the same `Item` the app reads;
- `.form(pairs)` for a form, and `.multipart(&form)` for files, as on [Request files](request-files.md);
- `.body(bytes)` for a body sent as it is.

`.send().await?` sends it and waits for the whole response.

### Responses

A `TestResponse` has `status()`, `headers()`, and the body as `text()?`, `body()` for the bytes, or `json()?`.
`json()` reads the body into any type that implements `Deserialize`: a `Value` to look into freely, as in `refuses_a_bad_token`, or a model such as `Item`, as in `creates_an_item`.

Comparing the whole `text()` against the expected JSON, as most of these pages do, also checks the order of the fields.

## Testing against other state

FastAPI's `app.dependency_overrides` replaces a dependency in tests, and Thermite doesn't have it yet.
What a test needs to change, such as the database, is given to the app as state, and a test builds the app with its own: `app()` can take the state as an argument.

## What is missing

The client has no cookie jar: a cookie the app sets isn't sent back by later requests, so a test sends the `Cookie` header itself.
There is no `base_url`, and requests go to paths, such as `/items/foo`.
