# Bigger applications

As an app grows, its routes are split into modules, each with a `Router` that the app includes.
This is FastAPI's `APIRouter` and `app.include_router`.

## Modules

A Rust crate is a tree of modules, much like a Python package is a tree of modules.
In a real project each module is a file of its own:

```text
src/
  main.rs           mod dependencies; mod items; mod users;
  dependencies.rs
  items.rs
  users.rs
```

`mod users;` in `main.rs` makes `src/users.rs` the module `users`.
Unlike Python, a file is only part of the crate once a `mod` line names it.

This page keeps the whole app in one file, so it can be run as one binary, and writes each module inline as `mod users { ... }`.
The contents are what the file would hold.

Everything in a module is private to it unless marked `pub`.
`users::router` is `pub`, so `main.rs` can call it, while the handlers and models stay inside.

## A router

```rust
{{#include ../../../docs_src/src/bin/bigger_applications.rs:users}}
```

A module starts with its own `use` lines: it doesn't see what the file around it imports.
`Router` has the same `get`, `post` and `routes` as `App`, and `.tag("users")` groups its routes under "users" in the docs.

`/users/me` sits beside `/users/{username}`, and as on [Path parameters](path-params.md), the fixed segment is tried first: `/users/me` answers `{"username":"fakecurrentuser"}`, and `/users/rick` answers `{"username":"rick"}`.

## A router with a prefix and a dependency

```rust
{{#include ../../../docs_src/src/bin/bigger_applications.rs:dependencies}}
```

```rust
{{#include ../../../docs_src/src/bin/bigger_applications.rs:items}}
```

`.prefix("/items")` puts every route of the router under `/items`, so `#[get("")]` is `/items` and `#[get("/{item_id}")]` is `/items/{item_id}`.
A path parameter in a prefix, such as `.prefix("/users/{user_id}")`, can be read by every route under it.

`use crate::dependencies::VerifiedToken` names the dependency by its path from the root of the crate, `crate`.
`TokenHeader` is `pub` as well, since `VerifiedToken` names it in `type Needs`, and a public item can't expose a private type.

In FastAPI, `APIRouter(dependencies=[Depends(get_token_header)])` adds the dependency to every route of the router.
Thermite doesn't have that yet, so each handler asks for it with `_: Depends<VerifiedToken>`, as on [Dependencies](dependencies.md).

With `X-Token: fake-super-secret-token`, `/items` answers `["plumbus","gun"]`, `/items/nope` answers 404 with `{"detail":"Item not found"}`, and with a wrong token, every route under `/items` answers 400:

```json
{"detail":"X-Token header invalid"}
```

## The app

```rust
{{#include ../../../docs_src/src/bin/bigger_applications.rs:imports}}

{{#include ../../../docs_src/src/bin/bigger_applications.rs:app}}
```

`include_router` adds the routes of a router to the app, under its prefix and tags.
A router can include other routers too, with its prefix and tags put in front of theirs.
Each route is checked as it is included, so a route whose path doesn't match its struct is an error from `app()`, as for a route added to the app directly.

`/` answers `Hello Bigger Applications!`.
`cargo run -p docs_src --bin bigger_applications` runs this page's app, and `main` is the same as in [First steps](first-steps.md).

## The docs

The routes appear in the docs under their tags: `GET /items/{item_id}` has `"tags": ["items"]`, and `GET /users/me` has `"tags": ["users"]`.
`/` has no tag, so Swagger UI lists it under "default".
The operation ID includes the prefix, `read_item_items__item_id__get`, as FastAPI builds it.

FastAPI also takes `tags=`, `responses=` and `dependencies=` on `include_router` itself.
In Thermite the tags and the prefix are set on the router.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/bigger_applications.rs:test}}
```
