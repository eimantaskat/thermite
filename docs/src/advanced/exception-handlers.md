# Exception handlers

[Handling errors](../tutorial/handling-errors.md) adds a handler for an error of your own and one that reshapes the 422.
This page replaces how every `HttpError` is answered, and says which errors reach a handler and which don't.

```rust
{{#include ../../../docs_src/src/bin/exception_handlers.rs:imports}}
```

## Replacing the HttpError handler

```rust
{{#include ../../../docs_src/src/bin/exception_handlers.rs:handler}}
```

FastAPI's example answers every `HTTPException` in plain text, by registering a handler for Starlette's `HTTPException`.
This is the same: the handler gets the `HttpError` and builds the response from its status and detail.

The detail is a JSON `Value`, so a text detail is taken as it is with `as_str()`, and any other value is written as JSON.
`extend` copies the error's headers onto the response, which matters for errors that carry one, such as the `Allow` of a 405 or the `WWW-Authenticate` of a 401.

```rust
{{#include ../../../docs_src/src/bin/exception_handlers.rs:app}}
```

`exception_handler` takes any function from `&E` to a response, a named `fn` as here or a closure as in the tutorial.
The type of `E` comes from the function's argument, so this one handles `HttpError` and nothing else.

`/items/3` now answers a 418 in plain text:

```text
content-type: text/plain; charset=utf-8

Nope! I don't like 3.
```

Without the handler it would answer `{"detail":"Nope! I don't like 3."}`, which `error.clone().into_response()` still builds, so a handler can fall back to the default answer for the errors it doesn't want to change.

`cargo run -p docs_src --bin exception_handlers` runs this page's app, and `main` is the same as in [First steps](../tutorial/first-steps.md).

## Which errors reach a handler

A handler sees an error that ends up in a response:

- an `Err` a route handler returns, as an `HttpError` or a `thermite::Error` wrapping any error;
- a rejection from an extractor or a dependency, such as the `RequestValidationError` of a 422, or the 401 of a [security scheme](../tutorial/security.md), which is an `HttpError`;
- the router's 404 for an unknown path and 405 for a known path with another method, which are `HttpError`s, so `/users` answers `Not Found` and `DELETE /items/1` answers `Method Not Allowed` here, both in plain text;
- a [WebSocket](websockets.md) handshake refused by a dependency or by `deny`.

It doesn't see:

- a panic, which answers a plain-text 500 `Internal Server Error`, as an exception does when it reaches Starlette's outermost middleware;
- the 413 of a body over the server's limit, which the server answers before the app sees the request;
- what a middleware answers by itself, since handlers run inside the middleware, which then see the replaced response, as in Starlette.

A handler is picked by the exact type of the error.
For a type with two handlers, the one added last is used, as in FastAPI.
An error no handler takes keeps its own answer: its status and detail for an `HttpError`, and a plain-text 500 for any other, which the app also logs through `tracing`.

## In the docs

The docs list the errors a route's types declare, such as the 422, with their default bodies.
Only a handler for `RequestValidationError` changes what the docs show, as the tutorial explains, since it is the one error every route that reads input can raise.
A handler for `HttpError` leaves the docs as they were, so here they still describe a 422 as JSON, which this handler doesn't change, and list no 418.

## What is missing

FastAPI's handlers also get the `Request`, to log its path or answer in the client's language; here they get only the error.
There is no `http_exception_handler` to import and call, but `error.clone().into_response()` gives the same default answer.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/exception_handlers.rs:test}}
```
