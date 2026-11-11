# Form data

An HTML form sends its fields as `application/x-www-form-urlencoded`, not as JSON.
`Form` reads such a body into a model, as `Annotated[Model, Form()]` does in FastAPI:

```rust
{{#include ../../../docs_src/src/bin/request_forms.rs:imports}}

{{#include ../../../docs_src/src/bin/request_forms.rs:login}}
```

The routes are listed in `app()`, and `main` is the same as in [First steps](first-steps.md):

```rust
{{#include ../../../docs_src/src/bin/request_forms.rs:app}}
```

`cargo run -p docs_src --bin request_forms` runs this page's app.

Sending `username=ada&password=secret&remember=on` answers:

```json
{"username":"ada","remember":true}
```

A wrong password answers 400:

```json
{"detail":"Incorrect password"}
```

Each field of the model is a field of the form.
The values arrive as text and are read as query parameters are, so `remember=on` is `true`, and a `Vec` field takes every value of a repeated field.
A field left out, or sent empty, is missing, and a field the model doesn't have is ignored.

A missing field is a 422 at `["body", "password"]`, whose `input` is what the form did send:

```json
{"detail":[{"type":"missing","loc":["body","password"],"msg":"Field required","input":{"username":"ada"}}]}
```

The docs list the body under its content type:

```json
"requestBody": {
  "content": {
    "application/x-www-form-urlencoded": {
      "schema": {"$ref": "#/components/schemas/Login"}
    }
  },
  "required": true
}
```

`Form` also reads a `multipart/form-data` body, which a form with a file input sends, and [Request files](request-files.md) covers that.

## Differences from FastAPI

FastAPI also lets each field be its own parameter, `username: str = Form()`.
In Thermite the fields are always gathered in one model.

A request has only one body, so a handler takes a `Form` or a `Json`, not both.
A route that asks for both is an error when it is added:

```text
`/both` declares more than one body
```

## Testing it

```rust
{{#include ../../../docs_src/src/bin/request_forms.rs:test}}
```

`.form(...)` sends the pairs urlencoded, as a browser would.
