# Middleware

A middleware sees every request before any route does, and every response after it, as in FastAPI.
This one adds the time each request took as a header:

```rust
{{#include ../../../docs_src/src/bin/middleware.rs:imports}}

{{#include ../../../docs_src/src/bin/middleware.rs:process_time}}
```

It is an `async fn` taking the `Request` and `next`, the counterpart of FastAPI's `call_next`.
`next.run(request).await` passes the request on, to the next middleware or the route, and gives back the response, which the middleware can change before returning it.
A middleware can also answer by itself without calling `next`, for example to refuse a request.

`next.run(request)` takes the request by value, so after that line the middleware no longer has it.
This is Rust's ownership: a value has one owner, and passing it to a function by value moves it there.
Reading anything from the request, such as its path, has to happen before the request is passed on.

`HeaderValue::from_str` fails on text that a header can't hold, such as a line break, so it returns a `Result`, and `if let Ok(value)` sets the header only when it succeeded.

## Adding it

```rust
{{#include ../../../docs_src/src/bin/middleware.rs:routes}}
```

```rust
{{#include ../../../docs_src/src/bin/middleware.rs:app}}
```

`App::middleware` adds a middleware for the whole app.
It also wraps the docs and the 404 for an unknown path, so `/missing` gets an `x-process-time` header too.
As in FastAPI, the middleware added last is the outermost and sees the request first.
Here `GZipMiddleware` runs first and `add_process_time` second.

`cargo run -p docs_src --bin middleware` runs this page's app, and `main` is the same as in [First steps](first-steps.md).

## GZip

`GZipMiddleware` is Starlette's, with the same settings.
It compresses a response when the client sends `Accept-Encoding: gzip` and the body is at least `minimum_size` bytes, 500 unless set and 1000 in this app.

Here `/report` is 2600 bytes, so it comes back with `Content-Encoding: gzip` and `Vary: Accept-Encoding`, while `/` is too small and comes back as it is.
A client that doesn't accept gzip gets `/report` uncompressed.

[CORS](cors.md) is the other middleware Thermite provides.
A middleware is often run by the gateway in front of the app instead, and then it can stay off here.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/middleware.rs:test}}
```

`.parse()?` turns the header's text into an `f64`, and fails the test if it isn't a number.
The type to parse into comes from `let seconds: f64`.
