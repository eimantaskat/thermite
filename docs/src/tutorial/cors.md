# CORS

A browser lets a page from one origin, such as `http://localhost:8080`, read an answer from another origin only when that answer allows it.
An origin is the scheme, host and port of a URL.
The CORS middleware adds the headers that allow it, as FastAPI's `CORSMiddleware` does:

```rust
{{#include ../../../docs_src/src/bin/cors.rs:imports}}

{{#include ../../../docs_src/src/bin/cors.rs:app}}
```

The settings have Starlette's names and defaults, and the [Middleware](middleware.md) page covers `App::middleware`.

- `allow_origins` lists the origins allowed; `"*"` among them allows any.
- `allow_any_method` and `allow_any_header` are `allow_methods=["*"]` and `allow_headers=["*"]`.
  `allow_methods([Method::GET, ...])` and `allow_headers([...])` list them one by one.
- `allow_credentials(true)` lets a page read the answer to a request sent with the browser's cookies, `credentials: "include"` in `fetch`.
- `expose_headers` lists response headers the page may read, and `max_age` says how long a browser keeps a preflight answer, 600 seconds unless set.

`cargo run -p docs_src --bin cors` runs this page's app, and `main` is the same as in [First steps](first-steps.md).

## Simple requests

A `GET` from `https://example.com` gets the answer with these headers:

```text
access-control-allow-credentials: true
access-control-allow-origin: https://example.com
vary: Origin
```

From an origin not in the list, the answer has no `access-control-allow-origin`, so the browser keeps it from the page.
The request still reached the route: CORS protects what a page can read, not the server.

## Preflight requests

Before a request a browser can't send from a plain form, such as a `PUT` or one with a JSON body, it asks first with an `OPTIONS` request carrying `Origin` and `Access-Control-Request-Method`.
The middleware answers it without reaching any route, with a plain-text `OK` and:

```text
access-control-allow-credentials: true
access-control-allow-methods: DELETE, GET, HEAD, OPTIONS, PATCH, POST, PUT, QUERY
access-control-allow-origin: https://example.com
access-control-max-age: 600
vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers, Access-Control-Request-Private-Network
```

From an origin not in the list, the answer is a 400 naming what was refused:

```text
Disallowed CORS origin
```

## Origins matched by code

Starlette's `allow_origin_regex` is `allow_origin_fn` here, a closure that gets the origin and returns whether it is allowed, beside the origins listed in `allow_origins`:

```rust
{{#include ../../../docs_src/src/bin/cors.rs:origin_fn}}
```

`|origin| ...` is a closure, Rust's `lambda`, and its body may be any expression.
Unlike a regex, the closure must check the whole origin itself: `ends_with("example.com")` without the dot would also allow `https://evilexample.com`.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/cors.rs:test}}
```
