# FastAPI to Thermite

This page puts FastAPI's names beside Thermite's, so an app can be carried over piece by piece.
Each row links to the page that covers it.

## One app in both

```python
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

app = FastAPI()


class Item(BaseModel):
    name: str
    price: float
    tags: list[str] = []


@app.get("/items/{item_id}")
async def read_item(item_id: int, q: str | None = None):
    if item_id == 0:
        raise HTTPException(status_code=404, detail="Item not found")
    return {"item_id": item_id, "q": q}


@app.post("/items", status_code=201)
async def create_item(item: Item) -> Item:
    return item
```

```rust
{{#include ../../../docs_src/src/bin/fastapi_to_thermite.rs:all}}
```

Both answer `/items/5?q=somequery` with `{"item_id":5,"q":"somequery"}` and `/items/0` with a 404 and `{"detail":"Item not found"}`.
`/items/5` answers `{"item_id":5,"q":null}`, as an unset `Option` is written as `null`.

What changes on the way:

- Parameters are read from a struct, one for the path and one for the query, instead of from the function's signature.
- The dictionary FastAPI returns becomes a struct of its own, `ItemRead`, so the docs know its fields.
- `raise` becomes `return Err(...)`, and the handler's return type says it can fail.
- `status_code=201` becomes a `(StatusCode, body)` pair; unlike FastAPI's, it isn't listed in the docs, as [Response models and status codes](../tutorial/response-model.md) explains.
- `uvicorn main:app` becomes the `main` function.

`cargo run -p docs_src --bin fastapi_to_thermite` runs it.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/fastapi_to_thermite.rs:test}}
```

## The app and routes

| FastAPI | Thermite |
| --- | --- |
| `app = FastAPI()` | `App::new()`, [First steps](../tutorial/first-steps.md) |
| `@app.get("/")` | `#[get("/")]` on the function, then `App::new().routes(routes![...])` |
| `@app.get(...)` with `responses=` | `App::get(path, handler.response_with_body::<T>(status, description))`, or `.response(status, description)` without a body, from the `Handler` trait, [Response models](../tutorial/response-model.md) |
| `FastAPI(title=..., version=...)` | `App::new().title(...).version(...)`, [Metadata and docs URLs](../tutorial/metadata.md) |
| `docs_url=`, `openapi_url=` | `.docs_path(...)`, `.openapi_path(...)` |
| `APIRouter(prefix=..., tags=[...])` | `Router::new().prefix(...).tag(...)`, [Bigger applications](../tutorial/bigger-applications.md) |
| `app.include_router(router)` | `App::include_router(router)?` |
| `uvicorn main:app` | `Server::bind(address).await?.serve(app).await?`, [Server settings](../advanced/server-settings.md) |
| `uvicorn --workers 4` | Nothing to set: one process runs a thread per core, [Concurrency and async](concurrency.md) |

## Requests

| FastAPI | Thermite |
| --- | --- |
| `class Item(BaseModel)` | `#[derive(Deserialize, Serialize, Model)] struct Item`, [Request body](../tutorial/body.md) |
| `tags: list[str] = []` | `#[model(default)] tags: Vec<String>` |
| `q: str \| None = None` | `q: Option<String>` |
| `item_id: int` in the path | `Path<T>`, [Path parameters](../tutorial/path-params.md) |
| `q: str` in the query | `Query<T>`, [Query parameters](../tutorial/query-params.md) |
| `item: Item` | `Json<Item>`, [Request body](../tutorial/body.md) |
| `Header()`, `Cookie()` | `Header<T>`, `Cookie<T>`, [Header and cookie parameters](../tutorial/header-cookie-params.md) |
| `Form()` | `Form<T>`, [Form data](../tutorial/request-forms.md) |
| `UploadFile` | `UploadFile` as a field of a `Form<T>` model, [Request files](../tutorial/request-files.md) |
| `request: Request` | A `FromRequest` of your own, [Using the request directly](../advanced/using-request-directly.md) |

## Responses and errors

| FastAPI | Thermite |
| --- | --- |
| `return item` with `response_model=Item` | Return `Json<Item>`, [Response models and status codes](../tutorial/response-model.md) |
| `status_code=201` | Return `(StatusCode::CREATED, Json(item))` |
| `Response`, `JSONResponse` | `Response`, built with `Response::new(Body::from(...))`, [Return a response directly](../advanced/response-directly.md) |
| `StreamingResponse` | `StreamingResponse`, [Stream data](../advanced/stream-data.md) |
| `EventSourceResponse` | `EventSourceResponse` of `ServerSentEvent`s, [Server-sent events](../advanced/server-sent-events.md) |
| `raise HTTPException(404, ...)` | `return Err(HttpError::new(StatusCode::NOT_FOUND, ...))`, [Handling errors](../tutorial/handling-errors.md) |
| `@app.exception_handler(E)` | `App::exception_handler(\|error: &E\| ...)`, [Exception handlers](../advanced/exception-handlers.md) |
| `RequestValidationError` | `RequestValidationError` |

## Dependencies and security

| FastAPI | Thermite |
| --- | --- |
| `Depends(get_db)` | `Depends<T>` for a `T` that implements `Dependency`, [Dependencies](../tutorial/dependencies.md) |
| A sub-dependency | `type Needs = (...)` in the `Dependency` |
| `OAuth2PasswordBearer(tokenUrl="token")` | `#[derive(SecurityScheme)]` with `#[security(oauth2_password, token_url = "token")]`, [Security](../tutorial/security.md) |
| `APIKeyHeader`, `HTTPBasic`, `HTTPBearer` | `#[security(api_key, header = ...)]`, `#[security(http_basic)]`, `#[security(http_bearer)]` |

## The rest of an app

| FastAPI | Thermite |
| --- | --- |
| `@app.middleware("http")` | `App::middleware(\|request: Request, next: Next\| ...)`, [Middleware](../tutorial/middleware.md) |
| A Starlette `BaseHTTPMiddleware` class | A type implementing `Middleware`, [Advanced middleware](../advanced/middleware.md) |
| `CORSMiddleware`, `GZipMiddleware` | `CorsMiddleware`, [CORS](../tutorial/cors.md); `GZipMiddleware`, [Middleware](../tutorial/middleware.md#gzip) |
| `BackgroundTasks` | `BackgroundTasks`, [Background tasks](../tutorial/background-tasks.md) |
| `app.state`, a module-level global | `App::state(value)` and `State<T>` |
| `lifespan=` | A `Resource` added with `App::resource::<T>()`, [Lifespan events](../advanced/events.md) |
| `app.mount("/static", StaticFiles(directory="static"))` | `App::mount("/static", StaticFiles::new("static"))?`, [Static files](../tutorial/static-files.md) |
| `@app.websocket("/ws")` | `#[websocket("/ws")]`, [WebSockets](../advanced/websockets.md) |
| `TestClient(app)` | `TestClient::start(app).await?`, [Testing](../tutorial/testing.md) |

## Habits to change

- **Handlers are `async fn`.** There is no `def` handler run in a thread pool; blocking work goes through `spawn_blocking`, as [Concurrency and async](concurrency.md) shows.
- **Mistakes in routes fail at startup.** Two routes with the same method and path, or a path parameter missing from the path, make `routes` return a `RouteError` before the server answers a request.
- **Mistakes in types fail at build time.** A handler argument Thermite can't read, or a response it can't document, is a compiler error, which [Rust in a handler](rust-in-a-handler.md#traits) helps to read.
- **A `String` is plain text.** `return "x"` in FastAPI sends the JSON string `"x"`; a handler returning a `String` sends `x` as plain text, and `Json(text)` sends JSON.

The behaviour that differs in smaller ways, and what Thermite doesn't have yet, is on the next page, [Differences from FastAPI](differences.md).
