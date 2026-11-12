# Return a response directly

A handler usually returns a `Json<T>`, a `String` or a [pair with a status](../tutorial/response-model.md), and Thermite builds the response from it.
It can also build the `Response` itself, as FastAPI's handlers can return a `Response` or a `JSONResponse`.
That is the way to send any media type, headers or cookies the return types don't cover.

```rust
{{#include ../../../docs_src/src/bin/response_directly.rs:imports}}
```

## A response of your own

```rust
{{#include ../../../docs_src/src/bin/response_directly.rs:legacy}}
```

`Response` is `http::Response<Body>`, the response type of the `http` crate, which hyper, reqwest and most of Rust's HTTP libraries share.
`Response::new(body)` makes a 200 without headers, and `headers_mut()` gives its headers to change.
Here it sends XML with `Content-Type: application/xml`, which the client receives exactly as written.

`Body` is what a response sends: `Body::from` takes a `String`, a `&'static str`, a `Vec<u8>` or `Bytes`, `Body::empty()` sends nothing, and a stream is the subject of [Stream data](stream-data.md).
`r#"..."#` is a raw string, which may hold `"` and line breaks without escapes, like a Python `"""..."""`.

Thermite sends the response as it is, without checking it against a model, as FastAPI does.

## Headers

```rust
{{#include ../../../docs_src/src/bin/response_directly.rs:headers}}
```

FastAPI adds headers through a `Response` parameter it merges into the answer.
Thermite has no such parameter: the handler turns its body into a response with `into_response()`, from the `IntoResponse` trait every return type implements, and changes it before returning it.
`GET /headers` answers `{"message":"Hello World"}` with `x-cat-dog: alone in the world`.

`insert` takes the header's name as a `&'static str` or as one of the constants in `thermite::http::header`, such as `CONTENT_TYPE`.
A name written as text is lowercased, and `insert` panics on one holding a character a header name can't, such as a space or a `:`.
`HeaderValue::from_static` panics in the same way on a value a header can't hold, such as one with a line break, which is fine for text written in the program; a value that comes from the request goes through `HeaderValue::from_str`, which returns a `Result`, as on [Middleware](../tutorial/middleware.md).

## Cookies

```rust
{{#include ../../../docs_src/src/bin/response_directly.rs:cookies}}
```

A cookie is a `Set-Cookie` header, and each cookie needs its own.
`append` adds a header beside any of the same name, where `insert` replaces them, so `/cookie` answers with both:

```text
set-cookie: fakesession=fake-cookie-session-value; Path=/; SameSite=lax
set-cookie: theme=dark; Path=/; SameSite=lax
```

FastAPI's `response.set_cookie(key, value)` writes the attributes for you.
Thermite has no helper for it yet, so the attributes are written by hand, such as `Max-Age=3600`, `HttpOnly` or `Secure`.
The [`cookie`](https://crates.io/crates/cookie) crate can build it, and then `HeaderValue::from_str(&cookie.to_string())` turns it into a header; a value that may hold characters such as `;` needs the crate's `percent-encode` feature and `cookie.encoded()`.

## Redirects and other statuses

```rust
{{#include ../../../docs_src/src/bin/response_directly.rs:redirect}}
```

A pair puts its status on the response, a `Response` included, so `/typer` answers a 307 with the `Location` header and an empty body.
The handler could also set it with `*response.status_mut() = StatusCode::TEMPORARY_REDIRECT`.
`*` writes through the reference that `status_mut()` returns, into the response.

FastAPI's `RedirectResponse`, `HTMLResponse` and `FileResponse` are not in Thermite yet: a redirect and HTML are built as above, and files are served by [`StaticFiles`](../tutorial/static-files.md).

## In the docs

```rust
{{#include ../../../docs_src/src/bin/response_directly.rs:app}}
```

A `Response` is decided as the handler runs, so nothing in its type says what it holds.
The docs show such a route with the default 200 of any JSON, as FastAPI does for a handler returning a `Response`, even when it sends XML:

```json
"200": {
  "description": "Successful Response",
  "content": {"application/json": {"schema": {}}}
}
```

`response_with_body::<Message>(status, description)` adds what the type can't show, here that `/headers` sends a `Message`, the counterpart of FastAPI's `responses=` on the route.
`#[get]` takes only the path, so a route with an added response goes through `App::get(path, handler)`, and `get_headers` has no attribute.
`cargo run -p docs_src --bin response_directly` runs this page's app, and `main` is the same as in [First steps](../tutorial/first-steps.md).

## Testing it

```rust
{{#include ../../../docs_src/src/bin/response_directly.rs:test}}
```
