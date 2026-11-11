# Path parameters

A path can hold parameters, written in braces as in FastAPI:

```rust
{{#include ../../../docs_src/src/bin/path_params.rs:imports}}

{{#include ../../../docs_src/src/bin/path_params.rs:item}}
```

The parameters are fields of a struct that derives `Model` and serde's `Deserialize`, and the handler asks for that struct wrapped in `Path`.
Where FastAPI reads `item_id: int` from the function's signature, Thermite reads `item_id: i64` from the struct.

`Path(ItemPath { item_id }): Path<ItemPath>` takes the argument apart where it is declared: the pattern on the left of the `:` unwraps the `Path` and then the struct, so the body uses `item_id` directly.
Rust allows such a pattern wherever it binds a name, in `let`, in `match` and in function arguments.
Writing `Path(path): Path<ItemPath>` and then `path.item_id` works too.

The `main` function is the same as in [First steps](first-steps.md), and `cargo run -p docs_src --bin path_params` runs this page's app.

## Types are checked

[http://127.0.0.1:8000/items/3](http://127.0.0.1:8000/items/3) answers:

```json
{"item_id":3}
```

The `3` came in as text, and Thermite parsed it into an `i64` before the handler ran.
[http://127.0.0.1:8000/items/foo](http://127.0.0.1:8000/items/foo) isn't an integer, so the handler isn't called and the answer is a 422 in FastAPI's shape:

```json
{"detail":[{"type":"int_parsing","loc":["path","item_id"],"msg":"Input should be a valid integer, unable to parse string as an integer","input":"foo"}]}
```

`loc` says where the problem is: the path parameter `item_id`.
`/items/4.2` fails the same way, since `4.2` is not an integer.
[Handling errors](handling-errors.md) covers the 422 body in full.

Integers are `i64` and floats `f64`; the other widths, such as `u32`, aren't supported yet.
A text parameter is a `String`.

The docs list the parameter with its type:

```json
"parameters": [
  {
    "name": "item_id",
    "in": "path",
    "required": true,
    "schema": {"type": "integer", "title": "Item Id"}
  }
]
```

## Fixed paths beside parameters

A fixed path like `/users/me` can sit beside `/users/{user_id}`:

```rust
{{#include ../../../docs_src/src/bin/path_params.rs:users}}
```

`/users/me` answers `{"user_id":"the current user"}`, and `/users/ada` answers `{"user_id":"ada"}`.

This differs from FastAPI, where routes are tried in the order they were added, so `/users/me` has to be declared first.
Thermite always tries the fixed segment first, in whatever order the routes are listed:

```rust
{{#include ../../../docs_src/src/bin/path_params.rs:app}}
```

## Mistakes caught at startup

A struct whose fields don't match the braces in the path is a mistake in the code, not in a request, so it is caught when the routes are added.
A route on `/files/{file_id}` whose struct names the field `id` makes `app()` return an error:

```text
path `/files/{file_id}` has no segment `{id}` for its path parameter `id`
```

Since `main` returns that error with `?`, the server never starts with a broken route.
A test that builds the app, like the one in First steps, catches it too.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/path_params.rs:test}}
```
