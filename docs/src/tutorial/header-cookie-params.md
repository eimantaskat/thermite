# Header and cookie parameters

Headers and cookies are read like path and query parameters, with `Header` and `Cookie`:

```rust
{{#include ../../../docs_src/src/bin/header_cookie_params.rs:imports}}

{{#include ../../../docs_src/src/bin/header_cookie_params.rs:models}}
```

```rust
{{#include ../../../docs_src/src/bin/header_cookie_params.rs:handler}}
```

The route is listed in `app()`, and `main` is the same as in [First steps](first-steps.md):

```rust
{{#include ../../../docs_src/src/bin/header_cookie_params.rs:app}}
```

`cargo run -p docs_src --bin header_cookie_params` runs this page's app.

## Underscores become hyphens

Header names are written with hyphens, `User-Agent`, which a Rust field name can't hold.
As in FastAPI, the field `user_agent` reads the header `user-agent`, and header names are compared without regard to case.
The docs list the header as `user-agent`.

Cookies keep their names as written: `ads_id` reads the cookie `ads_id`.

## Repeated headers

A `Vec<String>` field reads every value of a header sent more than once.
A request with

```text
User-Agent: curl/8.5.0
X-Token: foo
X-Token: bar
Cookie: theme=dark; ads_id=abc
```

answers:

```json
{"user_agent":"curl/8.5.0","x_token":["foo","bar"],"ads_id":"abc"}
```

The `theme` cookie isn't a field, so it's ignored, as an unknown query parameter is.
A request with none of them answers:

```json
{"user_agent":null,"x_token":[],"ads_id":null}
```

A required header or cookie that is missing is a 422 at `["header", "x-token"]` or `["cookie", "session_id"]`.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/header_cookie_params.rs:test}}
```
