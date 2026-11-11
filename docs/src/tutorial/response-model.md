# Response models and status codes

## The return type is the response model

FastAPI takes a `response_model`, or reads the return annotation, to filter and document what a handler sends.
In Thermite the return type does both: `Json<UserOut>` sends a `UserOut` and nothing else, and the docs show its schema.

A model for what comes in and another for what goes out keep a password from ever being sent back:

```rust
{{#include ../../../docs_src/src/bin/response_model.rs:imports}}

{{#include ../../../docs_src/src/bin/response_model.rs:users}}
```

`UserOut` has no `password` field, so no handler returning it can leak one.
The handler reads the password to check its length, answering 400 with an [`HttpError`](handling-errors.md) when it is too short, and then moves the other fields into a `UserOut`.

`user.username` moves the `String` out of `user` rather than copying it.
In Rust every value has one owner, here the handler's `user`; moving a field hands it to the new struct, and the compiler then refuses any later use of `user.username`, so no copy is needed and none is made.

Sending `{"username": "ada", "password": "correct horse", "email": "ada@example.com"}` answers:

```json
{"username":"ada","email":"ada@example.com","full_name":null}
```

The docs show `UserOut` as the 200 response:

```json
"200": {
  "description": "Successful Response",
  "content": {
    "application/json": {
      "schema": {"$ref": "#/components/schemas/UserOut"}
    }
  }
}
```

`cargo run -p docs_src --bin response_model` runs this page's app.

## What a handler can return

| Return type | Response | In the docs |
| --- | --- | --- |
| `Json<T>` | `T` as JSON, `Content-Type: application/json` | a 200 with `T`'s schema |
| `&'static str`, `String` | the text, `Content-Type: text/plain; charset=utf-8` | nothing |
| `()` | an empty 200 | a 200 without a body |
| `StatusCode` | that status with an empty body | nothing |
| `(StatusCode, body)` | the body with that status, unless the body is itself an error, such as a `Json` that failed to serialize | the body's errors only |
| `Result<T, E>` | `T` when it is `Ok`, `E` when it is `Err` (see [Handling errors](handling-errors.md)) | both sides |

A returned `String` is sent as plain text, where FastAPI's `return "x"` sends the JSON string `"x"`; `Json(text)` sends JSON.

`&'static str` is text written into the program, which lives as long as it runs (`'static`), so the handler can return it without copying it.

## Status codes

FastAPI's `status_code=201` is a status returned beside the body:

```rust
{{#include ../../../docs_src/src/bin/response_model.rs:status}}
```

`StatusCode` holds the named statuses, `StatusCode::CREATED` for 201, `StatusCode::NO_CONTENT` for 204 and so on.

A handler could choose the status as it runs, so Thermite can't tell from the type which one it sends, and the docs would list none.
`response_with_body` says it for the docs:

```rust
{{#include ../../../docs_src/src/bin/response_model.rs:app}}
```

`response_with_body::<Item>(status, description)` lists a response with `Item`'s schema; `response(status, description)` lists one without a body, such as a 204.
They come from the `Handler` trait, which every handler function implements. A trait's methods can only be called where the trait is imported, which is why `Handler` is in the `use` line.
The docs now list a 201 and no 200, beside the 413 and 422 any route with a body can answer:

```json
"201": {
  "description": "Successful Response",
  "content": {
    "application/json": {
      "schema": {"$ref": "#/components/schemas/Item"}
    }
  }
}
```

The `#[post]` attribute takes only the path for now, so a route that lists responses by hand is added with `.post(path, handler)`, which every method has: `.get`, `.put`, `.delete` and the rest.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/response_model.rs:test}}
```
