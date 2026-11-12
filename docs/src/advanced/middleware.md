# Advanced middleware

The [tutorial's middleware](../tutorial/middleware.md) is an `async fn`.
A middleware with settings of its own is a type instead, the way Starlette's middleware are classes that `app.add_middleware` builds with keyword arguments.
This page writes Starlette's `TrustedHostMiddleware`, which Thermite doesn't provide, as an example.

```rust
{{#include ../../../docs_src/src/bin/custom_middleware.rs:imports}}
```

## The Middleware trait

```rust
{{#include ../../../docs_src/src/bin/custom_middleware.rs:middleware}}
```

A middleware is any type that implements the `Middleware` trait, whose one method, `call`, takes the request and `next`, as the `async fn` did.
The difference is `&self`: `call` can read the middleware's own settings, here the hosts it trusts.
An `async fn(Request, Next) -> Response` is a middleware because Thermite implements the trait for every function of that shape.

A trait is like a Python protocol or abstract base class: it names methods a type must have.
`impl Middleware for TrustedHostMiddleware` is where this type provides them, apart from the struct, and any type can implement any number of traits that way.
The compiler checks the implementation when the program is built, so passing `App::middleware` a type that doesn't implement the trait doesn't compile, rather than failing on the first request.

The middleware reads the `Host` header, drops a port such as `:8000`, and passes the request on only for a host it trusts.
Any other request is answered at once, with Starlette's 400 `Invalid host header` in plain text, and no route sees it.
A host of `*.example.com` allows every subdomain of `example.com`, and `*` allows any host.

Middleware never sees a [WebSocket handshake](websockets.md#how-it-differs-from-http-routes), so unlike Starlette's, this check leaves an app's WebSocket routes open to any host.

`impl IntoIterator<Item = impl Into<String>>` lets `new` take an array of `&str`, a `Vec<String>` or anything else that gives strings one by one.
`split_once(':')` gives the parts before and after the first `:`, or `None` when there is none, and `map_or(host, ...)` keeps the whole host then.
`Some(host) if self.allows(host)` is a match arm with a guard: it matches a `Some` only when the condition after `if` holds as well.

## Adding it

```rust
{{#include ../../../docs_src/src/bin/custom_middleware.rs:app}}
```

`App::middleware` takes the middleware by value and keeps it for every request, as it does an `async fn`.
A middleware is shared by all requests at once, from several threads, so the trait asks it to be `Send + Sync`, which a struct of `String`s is.
State it has to change, such as a counter, goes behind a lock or an atomic, as the [state on Background tasks](../tutorial/background-tasks.md#state-shared-by-every-request) does.

`cargo run -p docs_src --bin custom_middleware` runs this page's app, and `main` is the same as in [First steps](../tutorial/first-steps.md).
`curl -H "Host: api.example.com" http://127.0.0.1:8000/` answers `Hello World`, and `curl http://127.0.0.1:8000/` answers `Invalid host header`, since its host is `127.0.0.1`.

## What a middleware can do

- Change the request before passing it on, through `request.headers_mut()` or `request.extensions_mut()`, as [Using the request directly](using-request-directly.md) does to hand a value to the routes.
- Answer by itself without calling `next`, as here.
- Change the response that `next.run` gives back, as the tutorial's does.
- Limit how long the rest takes: `next.run(request)` is a future like any other, so `tokio::time::timeout` can wrap it and answer a 504 when it runs out.

The body has been read whole when a middleware gets the request, so `request.body()` is the bytes the client sent; the server's [body limit](server-settings.md) has already been applied.
A response's body may be a [stream](stream-data.md), so a middleware that changes the body has to handle both, as `GZipMiddleware` does.

## What is missing

Starlette's `HTTPSRedirectMiddleware`, and `TrustedHostMiddleware` with its `www_redirect`, are not in Thermite.
Behind a gateway both are usually done there, as [Behind a proxy](behind-a-proxy.md) explains.
Nor is there a counterpart of a pure ASGI middleware, which sees the raw messages below the request, such as the body as it arrives.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/custom_middleware.rs:test}}
```

`TestClient` sends `Host: testserver` unless a request sets its own, as Starlette's does, so a test of an app with this middleware sets the header, or trusts `testserver` too.
