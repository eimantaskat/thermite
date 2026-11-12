# Thermite

Thermite is a web framework for building APIs in Rust, in the style of [FastAPI](https://fastapi.tiangolo.com/).
You declare what a handler takes, and Thermite reads it from the request, validates it, and documents it.

## Features

- **Typed requests**: path and query parameters, headers, cookies, JSON bodies, forms and file uploads become Rust types. A request that doesn't fit is answered with FastAPI's 422 error body, which lists each problem with where it is.
- **Automatic docs**: every app serves its OpenAPI schema at `/openapi.json` and Swagger UI at `/docs`, built from the same types the handlers use.
- **Dependencies**: values shared between handlers, such as the current user, are declared once and injected where they are asked for.
- **Security**: API keys, HTTP Basic and bearer tokens, shown in the docs with their own "Authorize" button.
- **The rest of an app**: routers, middleware (CORS and GZip come with it), background tasks, static files, streaming, server-sent events and WebSockets.
- **Testing**: `TestClient` sends requests to an app without opening a port.

## Requirements

Rust 1.99 or newer, with edition 2024.
Thermite runs on [tokio](https://tokio.rs/).

## Installation

Thermite isn't on crates.io yet, so it's added from GitHub.
In the `Cargo.toml` of a new binary crate (`cargo new my-api`):

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
thermite = { git = "https://github.com/eimantaskat/thermite" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

There are no releases yet: this follows `main`, which this guide is built from, and `cargo update -p thermite` brings a project up to it.

Then go on to [First steps](tutorial/first-steps.md), or, new to Rust, start with [Rust types intro](learn/rust-types.md).

## Coming from FastAPI

The names follow FastAPI's where they can: `Depends`, `Query`, `Header`, `TestClient`, `StaticFiles`, `BackgroundTasks`.
Where Python reads a parameter's type hint, Thermite reads a Rust type: a struct that derives `Model` plays the part of a Pydantic model.
[FastAPI to Thermite](learn/fastapi-to-thermite.md) puts the names side by side, and [Differences from FastAPI](learn/differences.md) lists where the behaviour differs.
