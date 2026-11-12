# Server settings

FastAPI apps are served by uvicorn, which takes its settings on the command line.
Thermite's server is part of the program, so its settings are written in `main`, on the `Server` before it serves the app.

```rust
{{#include ../../../docs_src/src/bin/server_settings.rs:imports}}

{{#include ../../../docs_src/src/bin/server_settings.rs:app}}
```

## The settings

```rust
{{#include ../../../docs_src/src/bin/server_settings.rs:settings}}
```

| Setting | Default | What it does |
| --- | --- | --- |
| `with_body_limit(bytes)` | 2 MiB | A request whose body is larger is answered with a 413 before the app sees it |
| `with_header_read_timeout(duration)` | 30 s | A connection that takes longer to send a request's headers is closed |
| `with_shutdown_timeout(duration)` | 30 s | How long a shutdown waits for requests and background tasks to finish |

Each method takes the server and gives it back, so they chain, and a later call replaces an earlier one.

The body limit counts the bytes as they arrive, so a client sending a larger body without a `Content-Length` is cut off as well.
The answer is `{"detail":"Content Too Large"}`, which the docs list for every route that reads a body.
Uploads are kept in memory, so this limit is also the largest file a client can [send](../tutorial/request-files.md).

The header read timeout also closes keep-alive connections that have been idle that long.
`with_header_read_timeout(None)` turns it off, which suits a server behind a gateway that keeps idle connections open for longer, as [Behind a proxy](behind-a-proxy.md) explains.

## Stopping gracefully

```rust
{{#include ../../../docs_src/src/bin/server_settings.rs:main}}
```

`serve` runs until the process is killed.
`serve_with_shutdown(app, shutdown_requested)` also takes a function returning a future, and stops when that future ends, here on Ctrl+C.

A graceful shutdown stops accepting connections and closes WebSockets with 1012, lets every request in progress finish and runs the background tasks that are left, and then stops the app's [resources](events.md).
What hasn't finished when the shutdown timeout passes is dropped.
The function is called a second time while shutting down, so a second Ctrl+C skips what is left of it, as in uvicorn.

`tokio::signal::ctrl_c`, which needs tokio's `signal` feature, fails only when the program can't listen for the signal; it then waits forever with `pending()`, since ending would stop the server at once.
In a container, `docker stop` sends `SIGTERM` rather than Ctrl+C's `SIGINT`; `tokio::signal::unix::signal(SignalKind::terminate())` waits for it on Linux.

`cargo run -p docs_src --bin server_settings` runs this page's app, and Ctrl+C stops it.

## Other ways to start

- `Server::bind(address)` takes `"127.0.0.1:8000"`, `"0.0.0.0:8000"` to listen on every network interface, or a `SocketAddr`; port `0` picks a free port, which `server.local_address()` then gives.
- `Server::from_std(listener)` serves on a `std::net::TcpListener` made elsewhere.

The server runs on the tokio runtime that `#[tokio::main]` starts, with one thread for each CPU core.
Every thread serves requests, so there is no counterpart of uvicorn's `--workers`: one process uses every core.

## The access log

The server logs one event for each request, with the client's address, the method, the path with its query, the HTTP version, the status and how long it took, like uvicorn's access log.
It goes through the [`tracing`](https://crates.io/crates/tracing) crate, under the target `thermite::access`, and stays silent until the program installs a subscriber, such as `tracing_subscriber::fmt::init()` from the [`tracing-subscriber`](https://crates.io/crates/tracing-subscriber) crate.

## What is missing

HTTP/2, TLS and Unix sockets are not supported; HTTPS is done by a gateway in front, as [Behind a proxy](behind-a-proxy.md) explains.
Nor are uvicorn's `--limit-concurrency`, `--timeout-keep-alive` apart from the header read timeout, or `--reload`; [`bacon`](https://crates.io/crates/bacon) rebuilds and restarts on changes.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/server_settings.rs:test}}
```

`TestClient::with_body_limit` can test the limit alone, but the timeouts and the shutdown need a real server, here on port `0`, with [`reqwest`](https://crates.io/crates/reqwest) as a dev-dependency for the client.
`tokio::spawn` runs the server in the background, and `Notify`, from tokio's `sync` feature, is a signal one task sends another.

The test adds a middleware that tells it when a request reaches the app, so it asks the server to stop only once `/slow` is running, however slow the machine.
It still gets `done`, and `serving.await??` waits for the server to return.
The first `?` is for the task, which fails if it panicked, and the second for what `serve_with_shutdown` returned.
The body limit is set to 16 bytes, since a server refusing a large body closes the connection while the client may still be sending it, which the client reports as an error instead of the 413.
