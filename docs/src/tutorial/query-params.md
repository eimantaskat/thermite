# Query parameters

Query parameters, the `key=value` pairs after the `?` in a URL, are declared the same way as path parameters, with `Query` in place of `Path`:

```rust
{{#include ../../../docs_src/src/bin/query_params.rs:imports}}

{{#include ../../../docs_src/src/bin/query_params.rs:paging}}
```

`#[model(default = 0)]` gives the field a default, as `skip: int = 0` does in FastAPI.
So [http://127.0.0.1:8000/items](http://127.0.0.1:8000/items) answers:

```json
{"skip":0,"limit":10}
```

and `/items?skip=20` answers `{"skip":20,"limit":10}`.

`Paging` derives both `Deserialize`, to be read from the request, and `Serialize`, to be sent back as JSON.
In FastAPI one Pydantic model does both; in Rust each direction is its own trait, and a type implements only the ones it needs.

`cargo run -p docs_src --bin query_params` runs this page's app.

## Optional parameters

A field of type `Option<String>` may be left out:

```rust
{{#include ../../../docs_src/src/bin/query_params.rs:optional}}
```

`Option<T>` is Rust's `T | None`: a value is either `Some(value)` or `None`.
Rust has no `null`, so the type says whether a value can be missing, and the compiler makes the handler deal with `None` before using it.

This handler reads `item_id` from the path and `q` and `short` from the query, each from the struct it belongs to.
`/items/foo` answers:

```json
{"item_id":"foo","q":null,"description":"This is an amazing item that has a long description"}
```

and `/items/foo?q=bar&short=true` answers:

```json
{"item_id":"foo","q":"bar","description":null}
```

`None` becomes `null` in JSON.

`#[model(default)]`, without a value, uses the type's empty value: `false`, `0`, `""` or an empty list.

## Booleans

As in FastAPI, a `bool` is read, in any case, from `1`, `true`, `t`, `yes`, `y` or `on`, and from `0`, `false`, `f`, `no`, `n` or `off`.
Anything else is a 422.

## Required parameters

A field with neither a default nor `Option` is required:

```rust
{{#include ../../../docs_src/src/bin/query_params.rs:required}}
```

`/search` without `needy` answers 422:

```json
{"detail":[{"type":"missing","loc":["query","needy"],"msg":"Field required","input":null}]}
```

## Lists

A `Vec<String>` field reads every value of a repeated parameter, and `#[model(default)]` makes it `[]` when there is none.
`/search?needy=x&tags=a&tags=b` answers:

```json
{"needy":"x","tags":["a","b"]}
```

A `Vec` is a growable list, like Python's `list`, holding values of one type.

## The app

The routes are listed in `app()`, and `main` is the same as in [First steps](first-steps.md):

```rust
{{#include ../../../docs_src/src/bin/query_params.rs:app}}
```

## Renaming

The fields follow serde's names, so `#[serde(rename = "item-query")]` on a field, or `#[serde(rename_all = "camelCase")]` on the struct, changes the name the request uses and the name the docs show.
Serde attributes that change how a value is read, such as `default` or `alias`, are refused at compile time; `#[model(default = ...)]` is the way to give a default.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/query_params.rs:test}}
```
