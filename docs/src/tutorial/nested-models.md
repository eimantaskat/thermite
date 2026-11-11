# Nested models

A field of a model can be a list, or another model, at any depth:

```rust
{{#include ../../../docs_src/src/bin/nested_models.rs:imports}}

{{#include ../../../docs_src/src/bin/nested_models.rs:models}}
```

- `tags: Vec<String>` is a list of strings, `list[str]` in Python; `#[model(default)]` makes it `[]` when it is left out.
- `image: Option<Image>` is another model, which may be left out.
- `items: Vec<Item>` is a list of models.

Every model in the tree derives `Model`, and the `Serialize` and `Deserialize` it needs.

```rust
{{#include ../../../docs_src/src/bin/nested_models.rs:handlers}}
```

`cargo run -p docs_src --bin nested_models` runs this page's app.

## Errors deep in the body

Each level is checked, and an error names the whole way to the value.
In this offer, the second item's image has no `name`:

```json
{
  "name": "Spring sale",
  "items": [
    {"name": "Foo", "price": 35.4},
    {"name": "Bar", "price": 12.0, "image": {"url": "https://example.com/bar.png"}}
  ]
}
```

```json
{
  "detail": [
    {
      "type": "missing",
      "loc": ["body", "items", 1, "image", "name"],
      "msg": "Field required",
      "input": {"url": "https://example.com/bar.png"}
    }
  ]
}
```

A number in `loc` is a position in a list, counted from 0.

## A list as the whole body

`Json<Vec<Image>>` reads a body that is a JSON array:

```json
[{"url": "https://example.com/a.png", "name": "A"}]
```

## In the docs

Each model is listed once under `components.schemas`, and the models that hold it refer to it:

```json
"Item": {
  "properties": {
    "name": {"title": "Name", "type": "string"},
    "price": {"title": "Price", "type": "number"},
    "tags": {
      "default": [],
      "items": {"type": "string"},
      "title": "Tags",
      "type": "array"
    },
    "image": {
      "anyOf": [{"$ref": "#/components/schemas/Image"}, {"type": "null"}],
      "title": "Image"
    }
  },
  "required": ["name", "price"],
  "title": "Item",
  "type": "object"
}
```

## A model inside itself

A model can hold itself, as a comment holds its replies:

```rust
{{#include ../../../docs_src/src/bin/nested_models.rs:comment}}
```

Every reply is checked like the comment itself, and an error in a reply of a reply is at `["body", "replies", 0, "replies", 0, "text"]`.
The `Vec` matters: it keeps the replies elsewhere in memory, while a struct holding a `Comment` directly would contain itself and have no size the compiler could work out.

## The app

The three routes are listed in `app()`, and `main` is the same as in [First steps](first-steps.md):

```rust
{{#include ../../../docs_src/src/bin/nested_models.rs:app}}
```

## Types without a schema yet

Sets (`HashSet`), maps (`HashMap`, Pydantic's `dict[str, float]`), enums, dates and UUIDs have no schema yet, so a model that uses one doesn't compile.
The compiler names the type and says it "has no schema, so it cannot be validated or documented".

`thermite::Value`, any JSON value, is accepted anywhere and documented as such.

## Testing it

The response is read back into an `Offer`, since it derives `Deserialize`:

```rust
{{#include ../../../docs_src/src/bin/nested_models.rs:test}}
```
