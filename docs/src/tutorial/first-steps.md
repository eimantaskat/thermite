# First steps

The smallest Thermite app looks like this:

```rust
{{#include ../../../docs_src/src/bin/first_steps.rs:all}}
```

Put it in `src/main.rs` of the crate from [Installation](../index.md#installation).

## Run it

```console
$ cargo run
```

The server listens on `http://127.0.0.1:8000` until you stop it with Ctrl+C.
Open [http://127.0.0.1:8000](http://127.0.0.1:8000) in a browser and you'll see:

```json
{"message":"Hello World"}
```

To run the copy in this repository instead, use `cargo run -p docs_src --bin first_steps`.

## The docs

Go to [http://127.0.0.1:8000/docs](http://127.0.0.1:8000/docs).
Swagger UI lists the one route, and "Try it out" sends a request to it from the page.

The page is drawn from the app's OpenAPI schema, served at [http://127.0.0.1:8000/openapi.json](http://127.0.0.1:8000/openapi.json).
Part of it describes `GET /`:

```json
"paths": {
  "/": {
    "get": {
      "summary": "Root",
      "operationId": "root__get",
      "responses": {
        "200": {
          "description": "Successful Response",
          "content": {
            "application/json": {
              "schema": {"$ref": "#/components/schemas/Message"}
            }
          }
        }
      }
    }
  }
}
```

Nothing in the code describes the route by hand: the summary comes from the function's name and the schema from the type it returns.

## Step by step

### The response model

```rust
{{#include ../../../docs_src/src/bin/first_steps.rs:model}}
```

`#[derive(...)]` asks the compiler to write code for a type.
`Serialize`, from [serde](https://serde.rs/), turns a `Message` into JSON; `Model`, from Thermite, describes its fields as a JSON schema for the docs.
Together they do what subclassing `pydantic.BaseModel` does in FastAPI.

### The handler

```rust
{{#include ../../../docs_src/src/bin/first_steps.rs:handler}}
```

`#[get("/")]` is the counterpart of `@app.get("/")`: this function answers `GET` requests for `/`.
Wrapping the value in `Json` says how to send it, with `Content-Type: application/json`.

A Rust string literal like `"Hello World"` is a `&str`, borrowed text that lives in the program itself; `.to_owned()` makes the `String` the struct owns.

### The app

```rust
{{#include ../../../docs_src/src/bin/first_steps.rs:app}}
```

`App` holds the routes.
Adding them can fail, for example when two routes have the same method and path, so it returns a `Result`: either the app or a `RouteError` saying which route is wrong.
Keeping this in a function of its own lets tests build the same app.

### Serving it

```rust
{{#include ../../../docs_src/src/bin/first_steps.rs:main}}
```

Rust's `async` code needs a runtime to run it, the role `asyncio` plays in Python; `#[tokio::main]` starts [tokio](https://tokio.rs/) and runs `main` on it.
`Server::bind` opens the port, and `serve` answers requests with the app, like `uvicorn main:app`.

The `?` after a call returns early with its error, the way an exception would leave the function.
`Box<dyn Error>` lets `main` return any kind of error, so a port that is already taken ends the program with a message.

## Testing it

The same app can be tested without opening a port:

```rust
{{#include ../../../docs_src/src/bin/first_steps.rs:test}}
```

`cargo test` runs it.
`TestClient` is the counterpart of FastAPI's, and [Testing](testing.md) covers it fully.
