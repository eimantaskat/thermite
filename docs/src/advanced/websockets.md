# WebSockets

A WebSocket keeps a connection open so the client and the server can both send messages whenever they like, as in a chat.
This page follows FastAPI's WebSockets page.

```rust
{{#include ../../../docs_src/src/bin/websockets.rs:imports}}
```

## A WebSocket route

```rust
{{#include ../../../docs_src/src/bin/websockets.rs:echo}}
```

`#[websocket("/ws")]` is FastAPI's `@app.websocket("/ws")`.
The handler takes a `WebSocket` first, the connection before its handshake is answered, and `socket.accept().await` accepts it, giving a `WebSocketConnection` to talk on.

`receive_text` waits for the next text message, and `send_text` sends one; `receive_bytes`, `send_bytes`, `receive_json` and `send_json` work the same way for binary and JSON messages.
Each returns `Err(WebSocketDisconnect)` once the connection is closed, which `?` passes on, so this loop ends when the client leaves and the handler returns that error.
A handler returns either `()` or `Result<(), WebSocketDisconnect>`.

A browser connects with JavaScript:

```javascript
const socket = new WebSocket("ws://localhost:8000/ws");
socket.onmessage = (event) => console.log(event.data);
socket.onopen = () => socket.send("hello");
```

and logs `Message text was: hello`.

## Path, query, cookies and dependencies

```rust
{{#include ../../../docs_src/src/bin/websockets.rs:dependency}}
```

After the `WebSocket`, a handler takes the same extractors as an HTTP route, read from the handshake request: `Path`, `Query`, `Header`, `Cookie`, `State` and [dependencies](../tutorial/dependencies.md).
A browser can't set headers on a WebSocket, which is why FastAPI's example reads a cookie or a `token` in the query instead.

When a dependency or an extractor of your own fails, the handshake is answered with its rejection and no connection is made: `/items/foo/ws` without a cookie or a token answers a 403 with `{"detail":"Not authenticated"}`.
FastAPI's example raises `WebSocketException(code=1008)` instead, which uvicorn answers with a 403 without a body.
A handler can also refuse by itself with `socket.deny(response)` in place of `accept`, for any `IntoResponse`, such as an `HttpError`.

A path, query, header or cookie value that fails validation answers a 403 with an empty body, as in FastAPI, where an HTTP route would answer 422.

With `?token=some-key-token&q=3`, sending `hi` gets three messages back:

```text
Session cookie or query token value is: some-key-token
Query parameter q is: 3
Message text was: hi, for item ID: foo
```

## Disconnects

```rust
{{#include ../../../docs_src/src/bin/websockets.rs:disconnect}}
```

FastAPI catches `WebSocketDisconnect` with `except`.
Here it is the `Err` of `receive_text`, and `match` takes it apart: `loop` gives back the value its `break` carries, so `disconnect` is the error that ended the loop, with the close code the client sent and its reason.
Closing with 1001, "going away", the client leaves `Client #7 left the chat with the code 1001` in the log, which `GET /chat/log` lists.

`let Ok(mut connection) = ... else { return; }` takes the connection when `accept` succeeds and returns otherwise, since this handler returns `()` and can't pass the error on with `?`.

FastAPI's chat example sends each message to every connected client, which needs a connection split in two, one task receiving while another sends.
Thermite's connection can't be split yet, so that example has no counterpart here.

```rust
{{#include ../../../docs_src/src/bin/websockets.rs:app}}
```

The chat route takes `State<ChatLog>`, so the app adds one with `.state`, and the app wouldn't start without it.
`cargo run -p docs_src --bin websockets` runs this page's app, and `main` is the same as in [First steps](../tutorial/first-steps.md).

## How it differs from HTTP routes

- Middleware never sees a handshake, as in FastAPI, where HTTP middleware skips WebSockets.
- [Exception handlers](exception-handlers.md) see a handshake refused by a dependency or by `deny`, so the 403 above could be reshaped by one, but not the empty 403 of a failed validation.
- WebSocket routes are left out of the docs, as FastAPI leaves them.
- When the server shuts down, it closes every WebSocket with 1012, "service restart", as uvicorn does.
- A message larger than 16 MiB closes the connection with 1009, uvicorn's default limit.

## What is missing

Uvicorn's pings and `ws_max_size`, compression (`permessage-deflate`), extra headers on accept, `WebSocketException` and `iter_text()` are not in Thermite yet, nor is splitting a connection to send and receive from two tasks.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/websockets.rs:test}}
```

`client.websocket(path).connect().await?` opens a WebSocket to the app, and the `TestWebSocket` it gives has the same `send_*` and `receive_*` methods as the handler's connection, plus `close(code, reason)`.
A refused handshake is `Err(TestWebSocketError::Denied(response))`, holding the response the app answered.
`.header(name, value)` before `connect` sends a header, such as a `Cookie`.
