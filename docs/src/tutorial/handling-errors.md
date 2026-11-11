# Handling errors

Python raises an exception to stop a handler and answer with an error.
Rust has no exceptions: a function that can fail returns a `Result`, either `Ok(value)` or `Err(error)`, and the caller decides what to do with the error.
A handler that can fail returns a `Result` too, and Thermite answers with whichever side it gets.

## HttpError

`HttpError` is the counterpart of FastAPI's `HTTPException`:

```rust
{{#include ../../../docs_src/src/bin/handling_errors.rs:imports}}

{{#include ../../../docs_src/src/bin/handling_errors.rs:not_found}}
```

`/items/foo` answers `{"item":"The Foo Wrestlers"}`, and `/items/bar` answers 404 with:

```json
{"detail":"Item not found"}
```

The detail can be any JSON value, as in FastAPI: anything that converts into a `thermite::Value`, such as a string or a `Value` holding an object.

`cargo run -p docs_src --bin handling_errors` runs this page's app.

### The `?` operator

`?` after a `Result` returns its error from the function, and an `Option` can be turned into a `Result` first with `ok_or_else`:

```rust
{{#include ../../../docs_src/src/bin/handling_errors.rs:header}}
```

This reads like FastAPI's `raise`: the rest of the handler runs only when the item was found.

### Headers

`with_header` adds a header to the error response; `/items-header/bar` answers 404 with `x-error: There goes my error`.

### In the docs

Which `HttpError` a handler returns is decided as it runs, so the docs list only the 200 and the 422.
A 404 can be listed by hand with `response(StatusCode::NOT_FOUND, "Item not found")`, as [Response models and status codes](response-model.md) shows.

## Your own errors

Any error type can be returned from a handler by converting it into `thermite::Error`, which `?` and `.into()` do.
A type is an error when it implements `Debug`, here derived, `Display`, for its message, and the standard `Error` trait.
To cross into `thermite::Error` it must also be safe to send between threads (`Send + Sync`) and own its data (`'static`), which a struct of `String`s is:

```rust
{{#include ../../../docs_src/src/bin/handling_errors.rs:unicorn}}
```

`impl Error for UnicornError {}` has an empty body: the trait's methods all have defaults, so declaring it is enough.

On its own, an error that is not an `HttpError` answers 500 with `Internal Server Error` in plain text, as an unhandled exception does in FastAPI.
To answer it differently, register an exception handler for its type, the counterpart of `@app.exception_handler(UnicornException)`:

```rust
{{#include ../../../docs_src/src/bin/handling_errors.rs:app}}
```

The handler is a closure, `|error: &UnicornError| ...`, a function written inline, like a Python `lambda` that can hold several statements.
It borrows the error (`&`), which Thermite still owns and lends to it, and returns anything a route handler can return.

`/unicorns/yolo` now answers 418 with:

```json
{"message":"Oops! yolo did something. There goes a rainbow..."}
```

## The 422 body

A request that doesn't fit what the handler asks for never reaches the handler, and is answered with 422, such as this one from the [Path parameters](path-params.md) app for `/items/foo`:

```json
{"detail":[{"type":"int_parsing","loc":["path","item_id"],"msg":"Input should be a valid integer, unable to parse string as an integer","input":"foo"}]}
```

`detail` lists every problem with the request, each with:

- `type`: what kind of problem, using Pydantic's names (`missing`, `int_parsing`, `json_invalid`, ...)
- `loc`: where it is, starting with `path`, `query`, `header`, `cookie` or `body`, then the field names and list positions on the way to it; for a body that isn't valid JSON, the position where it breaks
- `msg`: a message for people, the same as Pydantic's
- `input`: the value that was sent; for a missing parameter `null`, and for a missing body field the object it is missing from
- `ctx`: for some types, more about the problem, worded by Thermite rather than by Pydantic

Since the shape is FastAPI's, a client written for a FastAPI service reads these errors as they are.
The docs list it under the `HTTPValidationError` schema, as FastAPI does.

### Changing it

An exception handler for `RequestValidationError` replaces the 422 for every route.
This app serves `read_item` from [Path parameters](path-params.md), with `item_id: i64`:

```rust
{{#include ../../../docs_src/src/bin/handling_errors_validation.rs:imports}}

{{#include ../../../docs_src/src/bin/handling_errors_validation.rs:app}}
```

`error.errors()` gives the entries of `detail`, each a `Value` read with `["loc"]` and `["msg"]`, and `/items/foo` now answers 400 with:

```text
["path","item_id"]: Input should be a valid integer, unable to parse string as an integer
```

The docs no longer list a 422, and since a status in `(StatusCode, body)` is chosen as the handler runs, they don't list the 400 either.

`cargo run -p docs_src --bin handling_errors_validation` runs this example.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/handling_errors.rs:test}}
```
