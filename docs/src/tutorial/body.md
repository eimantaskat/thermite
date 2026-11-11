# Request body

A client sends data to an API in the request body, usually as JSON.
The body's shape is a struct that derives `Model`:

```rust
{{#include ../../../docs_src/src/bin/body.rs:imports}}

{{#include ../../../docs_src/src/bin/body.rs:model}}
```

This is the counterpart of a Pydantic model:

```python
class Item(BaseModel):
    name: str
    description: str | None = None
    price: float
    tax: float | None = None
```

`name` and `price` are required; `description` and `tax` may be left out, and are then `None`.

The handler asks for it wrapped in `Json`:

```rust
{{#include ../../../docs_src/src/bin/body.rs:create}}
```

`Json` works both ways: as an argument it reads the body, and as the return type it writes one.
`cargo run -p docs_src --bin body` runs this page's app.

## What Thermite does with it

Sending this to `POST /items`:

```json
{"name": "Foo", "price": 45.2}
```

answers:

```json
{"name":"Foo","description":null,"price":45.2,"tax":null}
```

Thermite reads the body as JSON, checks it against the model, and only then builds the `Item` and calls the handler.
A body that doesn't fit is answered with every problem at once:

```json
{"description": "A very nice Item", "price": "cheap"}
```

```json
{
  "detail": [
    {
      "type": "missing",
      "loc": ["body", "name"],
      "msg": "Field required",
      "input": {"description": "A very nice Item", "price": "cheap"}
    },
    {
      "type": "float_parsing",
      "loc": ["body", "price"],
      "msg": "Input should be a valid number, unable to parse string as a number",
      "input": "cheap"
    }
  ]
}
```

Like Pydantic in its default mode, a number sent as a string, such as `"45.2"`, is accepted.

A body that isn't valid JSON is reported with the position where it breaks:

```json
{"name": "Foo",}
```

```json
{
  "detail": [
    {
      "type": "json_invalid",
      "loc": ["body", 15],
      "msg": "JSON decode error",
      "input": {},
      "ctx": {"error": "trailing comma"}
    }
  ]
}
```

The `ctx.error` message comes from the Rust JSON parser, so it is worded differently from Python's.

## In the docs

The model is listed once under `components.schemas`, and every route that uses it refers to it there:

```json
"Item": {
  "properties": {
    "name": {"title": "Name", "type": "string"},
    "description": {
      "anyOf": [{"type": "string"}, {"type": "null"}],
      "title": "Description"
    },
    "price": {"title": "Price", "type": "number"},
    "tax": {
      "anyOf": [{"type": "number"}, {"type": "null"}],
      "title": "Tax"
    }
  },
  "required": ["name", "price"],
  "title": "Item",
  "type": "object"
}
```

It has the shape FastAPI writes for the same Pydantic model, so tools that read FastAPI's docs read these too.
Two details differ: FastAPI adds `"default": null` to the optional fields, and since `Item` is both read and returned here, it lists the model twice, as `Item-Input` and `Item-Output`.

## Path, query and body together

A handler can ask for each of them:

```rust
{{#include ../../../docs_src/src/bin/body.rs:update}}
```

`PUT /items/5?q=x` with `{"name": "Foo", "price": 45.2}` answers:

```json
{"item_id":5,"q":"x","item":{"name":"Foo","description":null,"price":45.2,"tax":null}}
```

FastAPI decides where a parameter comes from by its type and name; here the wrapper says it, so `Path`, `Query` and `Json` can come in any order.
A route has at most one body: two `Json` arguments make `app()` return an error.

Both routes are listed in `app()`, and `main` is the same as in [First steps](first-steps.md):

```rust
{{#include ../../../docs_src/src/bin/body.rs:app}}
```


## Checks a model can't express yet

Pydantic's `Field(gt=0)` and `max_length`, and FastAPI's `Query(min_length=3)`, have no counterpart yet.
Until they do, check the value in the handler and answer with an error, as [Handling errors](handling-errors.md) shows.

## Testing it

`TestRequest::json` sends a value as the JSON body, so the test builds an `Item` rather than writing the JSON by hand:

```rust
{{#include ../../../docs_src/src/bin/body.rs:test}}
```
