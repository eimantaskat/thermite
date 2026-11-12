# Lifespan events

Some things have to be ready before the first request and released after the last one: a model loaded from disk, a database pool, a client for another service.
FastAPI does it with a `lifespan` function that runs up to its `yield` on startup and after it on shutdown.
In Thermite each of those things is a resource.

```rust
{{#include ../../../docs_src/src/bin/events.rs:imports}}
```

## A resource

```rust
{{#include ../../../docs_src/src/bin/events.rs:resource}}
```

A resource is a type that implements the `Resource` trait:

- `type Needs` lists what it needs to start, as `State<T>`, or a tuple of them, like a [dependency](../tutorial/dependencies.md)'s `Needs`; `()` needs nothing;
- `type Error` is the error its start can fail with, any type implementing the standard `Error` trait that is also `Send + Sync + 'static`, as for [your own errors](../tutorial/handling-errors.md#your-own-errors);
- `start` builds it from what it needs;
- `stop`, left out here, runs on shutdown, and does nothing unless the resource writes one.

`predict: fn(f64) -> f64` is a field holding a function, as a Python attribute can hold one, and `(model.predict)(input.x)` calls it.
The parentheses tell Rust to call the field rather than look for a method named `predict`.

## Adding it to the app

```rust
{{#include ../../../docs_src/src/bin/events.rs:app}}
```

`.resource::<MlModel>()` adds the resource by its type, since the app builds it itself.
`::<MlModel>` names the type that a generic function works on when nothing in the arguments says it, here with no arguments at all.

Once started, a resource is [state](../tutorial/background-tasks.md#state-shared-by-every-request): handlers, dependencies and later resources take it as `State<MlModel>`.
`GET /predict?x=2` answers `{"result":84.0}`.

## When it runs

The server starts the resources before it accepts the first connection, in the order they were added, so each can need the ones added before it.
On a graceful shutdown, once the last request has been answered and the last [background task](../tutorial/background-tasks.md) has run, it calls each one's `stop` in the reverse order.
A graceful shutdown needs `serve_with_shutdown`, as on [Server settings](server-settings.md#stopping-gracefully).
This page's `main` uses `serve`, which runs until the process is killed, and then no `stop` runs; nor does it when a second Ctrl+C forces the shutdown.

In Rust, a value is freed when its owner goes away, here when the app is dropped, so `stop` isn't needed to clear a model the way FastAPI's example clears its dictionary.
It is for work that has to be awaited, such as closing a pool's connections politely or flushing a buffer.
`stop` takes `&self` because the resource is shared behind an `Arc`, which a task a handler started may still hold, so the app can't hand it over by value.

## When it fails

```rust
{{#include ../../../docs_src/src/bin/events.rs:main}}
```

`main` reads the settings, here the model's name from the `MODEL_NAME` environment variable, and hands them to `app`.
`env::var` returns an `Err` when the variable isn't set, and `unwrap_or_else` takes the default then.

When a resource fails to start, the server starts no others, stops those already started, and `serve` returns the error, so the program exits before accepting any request:

```text
Error: Startup(Resource { resource: "events::MlModel", source: Custom { kind: Other, error: "no model named unknown" } })
```

`ServerError::Startup` holds a `StartupError`, whose message is `the resource events::MlModel failed to start`, with the resource's own error as its source.

Before starting anything, the server also checks that every `State` the routes, dependencies and resources need is there, and that each resource needs only state added before it.
A mistake there is a `StartupError` naming what is missing and who needs it, so it shows on startup rather than as a 500 on the first request.

`cargo run -p docs_src --bin events` runs this page's app.

## What is missing

FastAPI's deprecated `@app.on_event("startup")` and `"shutdown"` hooks have no counterpart, and a resource's `stop` returns nothing, so a failure there can only be logged.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/events.rs:test}}
```

`TestClient::start` runs the startup and `client.stop()` the shutdown, as entering and leaving FastAPI's `with TestClient(app):` do.
A failed startup is `Err(ServerError::Startup(error))`.

`app` takes its settings as an argument, so each test builds the app with the settings it needs, and the model with them.
This is how a test gives the app a different database or service, which FastAPI does with `dependency_overrides`.

`error.source()` is the error that caused this one, like Python's `__cause__`.
