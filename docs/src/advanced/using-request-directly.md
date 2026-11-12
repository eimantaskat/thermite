# Using the request directly

FastAPI's handlers can take the Starlette `Request` and read anything from it.
A Thermite handler doesn't take the request itself: what it reads, it reads through an extractor, such as `Path`, `Header` or a [dependency](../tutorial/dependencies.md).
For anything those don't cover, you write an extractor of your own, which gets the whole request.

```rust
{{#include ../../../docs_src/src/bin/using_request_directly.rs:imports}}
```

## An extractor of your own

```rust
{{#include ../../../docs_src/src/bin/using_request_directly.rs:extractor}}
```

An extractor is a type that implements the `FromRequest` trait.
`from_request` gets the `Request` and builds the value, or fails with its `Rejection`, which the client gets as the answer.
Reading the method, the path and a header can't fail, so the rejection is `Infallible`, a type with no values, which tells the compiler the `Err` never happens.

`Request` is `http::Request<Bytes>`, the `http` crate's request with its body already read:

- `request.method()`, `request.uri()` with `path()` and `query()`, and `request.headers()`;
- `request.body()`, the bytes the client sent;
- `request.extensions()`, values that middleware put there, as the next section does.

The second argument, `&Arguments`, holds the path, query, header and cookie values the route declared, already validated; an extractor that reads only the request ignores it with `_`.

A handler takes the extractor as an argument, in any position: `info: RequestInfo`.

## A value from a middleware

FastAPI's middleware hand values to handlers through `request.state`.
Here a middleware puts them in the request's extensions, a map holding one value of each type, and an extractor takes them out:

```rust
{{#include ../../../docs_src/src/bin/using_request_directly.rs:request_id}}
```

`extensions().get::<RequestId>()` looks the value up by its type, and `.copied()` copies it out of the request, which `#[derive(Clone, Copy)]` allows for a type as small as one number.
If no middleware put one there, it is a mistake in the app rather than in the request, so the extractor answers a 500.

```rust
{{#include ../../../docs_src/src/bin/using_request_directly.rs:app}}
```

The middleware here is a closure, `move |mut request, next| ...`, which owns the counter, so each request gets the next number.
It returns `next.run(request)` itself, a future, instead of an `async` block, since it has nothing to do after the route answers.

`AtomicI64` is a number that many threads can change at once without a lock: `fetch_add(1, ...)` adds one and gives the value from before.
`Ordering::Relaxed` asks only that the addition itself is never lost, which is all a counter needs.
The closure is the counter's only owner and changes it only through `&`, so it needs no `Arc` or lock.
A middleware can't take [`State`](../tutorial/background-tasks.md#state-shared-by-every-request), so it holds what it needs itself.

`/items/foo` answers:

```json
{"item_id":"foo","method":"GET","path":"/items/foo","user_agent":null,"request_id":1}
```

and without the middleware, the route answers a 500 with `{"detail":"No request id was set"}`.

`cargo run -p docs_src --bin using_request_directly` runs this page's app, and `main` is the same as in [First steps](../tutorial/first-steps.md).

## In the docs

An extractor of your own adds nothing to the docs, since the docs can't know what it reads.
A value the docs should show, such as a header the client has to send, belongs in a `Header` model, which a [dependency](../tutorial/dependencies.md) can then read.
An extractor can also add to the route's description and responses through the trait's `describe` method, which the built-in extractors use.

## What is missing

The client's address, FastAPI's `request.client.host`, is known to the server but not passed to the app yet.
There is no `request.url_for` either, nor an extractor for the extensions, which every app writes as above.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/using_request_directly.rs:test}}
```
