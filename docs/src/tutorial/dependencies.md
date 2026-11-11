# Dependencies

A dependency is a value that Thermite builds before the handler runs, from what it reads from the request.
It is FastAPI's `Depends`: the code that builds the value is written once and shared by every route that needs it.

```rust
{{#include ../../../docs_src/src/bin/dependencies.rs:imports}}

{{#include ../../../docs_src/src/bin/dependencies.rs:pagination}}
```

The routes are listed in `app()`, and `main` is the same as in [First steps](first-steps.md):

```rust
{{#include ../../../docs_src/src/bin/dependencies.rs:app}}
```

`cargo run -p docs_src --bin dependencies` runs this page's app.

`/items?page=3&size=5` answers `{"skip":10,"limit":5}`, and `/users` answers `{"skip":0,"limit":10}`.

## Writing one

A dependency is a type that implements the `Dependency` trait.
A trait is a set of functions that a type promises to provide, close to an abstract base class or a `typing.Protocol`, and `impl Dependency for Pagination` keeps that promise for `Pagination`.
Thermite calls `resolve` whenever a handler asks for `Depends<Pagination>`.

The trait also has two associated types, `type Needs` and `type Rejection`.
These are types chosen by each implementation, so the trait can talk about them without knowing them in advance:

- `Needs` is what the dependency reads from the request.
  It is anything a handler could take, such as `Query<PageQuery>` here, or several of them in a tuple.
- `Rejection` is what the dependency answers with when `resolve` returns an `Err`.
  The handler is then not called.
  A dependency that never fails uses `Infallible`, a type with no values, so the compiler knows the `Err` case can't happen.

`/items?page=0` answers 400:

```json
{"detail":"page and size start at 1"}
```

`resolve` is an `async fn`, so a dependency can await a database or another service.
The value must be `Clone`: Thermite builds it once per request and hands a copy to everything that asks for it, as the next section explains.

The docs list what the dependency reads as the route's own parameters:

```json
"parameters": [
  {
    "name": "page",
    "in": "query",
    "required": false,
    "schema": {"type": "integer", "default": 1, "title": "Page"}
  },
  {
    "name": "size",
    "in": "query",
    "required": false,
    "schema": {"type": "integer", "default": 10, "title": "Size"}
  }
]
```

FastAPI's first dependency example only groups `q`, `skip` and `limit`.
In Thermite a `Query` struct already does that, so a `Dependency` is worth writing only when it does some work, as `Pagination` does.

## Sub-dependencies

A dependency can need other dependencies:

```rust
{{#include ../../../docs_src/src/bin/dependencies.rs:sub_dependencies}}
```

`TermOrCookie` needs the `SearchTerm` dependency and a cookie.
`/search?q=iron` answers `{"q_or_cookie":"iron"}`.
Without `q`, the `last_query` cookie is used: a request with `Cookie: last_query=copper` answers `{"q_or_cookie":"copper"}`.

As in FastAPI, a dependency runs only once per request, however many of the handler's arguments and other dependencies ask for it.
Unlike FastAPI there is no `use_cache=False` to opt out yet.

## Dependencies that guard a route

A handler can ask for a dependency only to have it checked, and ignore its value:

```rust
{{#include ../../../docs_src/src/bin/dependencies.rs:guard}}
```

`_` is a pattern that accepts the value without naming it.
Unlike Python's `_`, which is an ordinary variable that keeps the value, it binds nothing, and the value is dropped at once.
The dependency still runs: with the right key `/reports` answers `the reports`, and with `X-Key: guess` it answers 400:

```json
{"detail":"X-Key header invalid"}
```

Without the header, the answer is a 422 at `["header", "x-key"]`, as it would be if the handler read the header itself:

```json
{"detail":[{"type":"missing","loc":["header","x-key"],"msg":"Field required","input":null}]}
```

This is how FastAPI's `dependencies=[Depends(verify_key)]` on a route is written in Thermite.
FastAPI also takes `dependencies=` for a whole router or app.
Thermite doesn't have that yet, so each handler asks for the dependency, as [Bigger applications](bigger-applications.md) shows.

## What is missing

Thermite has no dependencies with `yield`, which in FastAPI run code after the response, such as closing a database session.
A database pool is shared through the app's state instead, which [Background tasks](background-tasks.md) introduces.
Overriding a dependency in tests, FastAPI's `app.dependency_overrides`, isn't there either.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/dependencies.rs:test}}
```
