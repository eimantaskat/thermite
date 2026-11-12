# Differences from FastAPI

Thermite follows FastAPI's behaviour where it can: the same 422 body, the same status codes, the same names.
This page lists where it doesn't, so a client written for a FastAPI service, or a habit carried over from one, doesn't meet them by surprise.
Each item links to the page that explains it.

## Types and validation

- Integers are `i64` and floats `f64`; the other widths, such as `u32`, can't be used in a model yet. [Path parameters](../tutorial/path-params.md)
- Pydantic's `Field(gt=0)`, `max_length` and FastAPI's `Query(min_length=3)` have no counterpart yet, so such checks go in the handler. [Request body](../tutorial/body.md#checks-a-model-cant-express-yet)
- Sets, maps, enums, dates and UUIDs have no schema yet, so a model holding one doesn't compile. [Nested models](../tutorial/nested-models.md#types-without-a-schema-yet)
- Each form field is part of a `Form<T>` model; FastAPI also takes them one by one, `username: str = Form()`. [Form data](../tutorial/request-forms.md)

## Responses

- A handler returning a `String` sends it as plain text, where FastAPI's `return "x"` sends the JSON string `"x"`. [Response models and status codes](../tutorial/response-model.md)
- A status chosen with `(StatusCode, body)` isn't listed in the docs, where FastAPI's `status_code=` is, and `#[post]` takes only the path, so a status listed by hand goes through `App::post(path, handler.response(...))`. [Response models and status codes](../tutorial/response-model.md)
- There is no `Response` parameter to add headers or cookies to, nor `set_cookie`; the handler builds a `Response` instead. `RedirectResponse`, `HTMLResponse` and `FileResponse` aren't there either. [Return a response directly](../advanced/response-directly.md)
- A handler can't stream by yielding; it returns a `StreamingResponse` or an `EventSourceResponse` built from a stream. Both are listed in the docs as a 200 without content, where FastAPI lists a streamed response as JSON and events as `text/event-stream` with a schema for them. [Stream data](../advanced/stream-data.md), [Server-sent events](../advanced/server-sent-events.md)

## Errors

- Exception handlers get only the error, not the `Request`, and there is no `http_exception_handler` to import; `error.clone().into_response()` gives the default answer. [Exception handlers](../advanced/exception-handlers.md)
- A `RequestValidationError` handler that answers with a `(StatusCode, body)` pair leaves the docs listing no error for an invalid request. [Handling errors](../tutorial/handling-errors.md)

## Dependencies

- Dependencies with `yield`, which run code after the response, aren't there; a database pool lives in the app's state instead. [Dependencies](../tutorial/dependencies.md)
- Nor are `dependency_overrides`, `use_cache=False`, or `dependencies=` on a router, the app or `include_router`, so each handler asks for such a dependency itself. [Bigger applications](../tutorial/bigger-applications.md)

## Security

- OAuth2 scopes (`SecurityScopes`), OpenID Connect and `OAuth2AuthorizationCodeBearer` aren't there yet. [Security](../tutorial/security.md)
- Nor is `OAuth2PasswordRequestForm`, so a login reads its own `Form<T>` model, nor are the `refreshUrl`, `bearerFormat` and basic `realm` settings.

## Forms and files

- An upload is kept in memory, never on disk as FastAPI's `UploadFile` does with a large one, so the body limit, 2 MiB by default, is also the largest file a client can send. [Request files](../tutorial/request-files.md)
- There is no `bytes` parameter for a file's content; it is `UploadFile::content`.
- A form model holding files is listed in the docs under `multipart/form-data`, where FastAPI lists it under `application/x-www-form-urlencoded`, which can't carry a file.
- A multipart body cut short answers 400, where Starlette reads the parts before the cut. A preamble before the first boundary is skipped, where Starlette refuses it. A filename or field name that isn't UTF-8 isn't read as latin-1, and a `charset` on the request's content type is ignored.

## Middleware

- `CorsMiddleware` takes `allow_origin_fn(|origin| ...)` in place of `allow_origin_regex`, and the function must check the whole origin itself. An origin that isn't UTF-8 matches only `"*"`. [CORS](../tutorial/cors.md)
- `GZipMiddleware` compresses an empty stream, into 20 bytes, where Starlette sends it as it is. [Middleware](../tutorial/middleware.md#gzip)
- Starlette's `HTTPSRedirectMiddleware` and `TrustedHostMiddleware` aren't there, nor is a pure ASGI middleware that sees the body as it arrives. Middleware never sees a WebSocket handshake, so one checking hosts or credentials leaves the WebSocket routes open. [Advanced middleware](../advanced/middleware.md)

## Static files

- The `ETag` is the file's modification time in nanoseconds and its size, in hex, in the format nginx uses, where Starlette hashes them with MD5. [Static files](../tutorial/static-files.md)
- The content type comes from a table built in, the defaults of Python 3.14, where Python also reads the system's `mime.types` or the Windows registry; `.tar.gz` is `application/gzip`, where Python says `application/x-tar`.
- With `html`, the redirect to a directory's trailing slash has a `Location` of the path only, where Starlette writes the whole URL from the `Host` header.
- `/static` without its slash goes straight to the file lookup, where Starlette's router first redirects it to `/static/`.
- Routes are tried before mounts, wherever a mount was added, and the deepest mount answers first, where Starlette goes by the order they were added.
- Only `StaticFiles` can be mounted, not a sub-application.

## WebSockets

- A handshake refused by a dependency is answered with the dependency's rejection, such as a 403 with a JSON body, where FastAPI's example raises `WebSocketException`, answered with a 403 without one. A path, query, header or cookie that fails validation answers an empty 403, as in FastAPI. [WebSockets](../advanced/websockets.md)
- Uvicorn's pings, `ws_max_size` (messages are limited to 16 MiB and closed with 1009), compression, extra headers on accept, `WebSocketException` and `iter_text()` aren't there yet, nor is splitting a connection to send and receive from two tasks.

## The request

- The client's address, `request.url_for` and `request.state` aren't there yet; a value a middleware stores goes in the request's extensions, read by an extractor of your own. [Using the request directly](../advanced/using-request-directly.md)
- Nor are `root_path`, the `servers` list in the docs, or reading the forwarded headers of trusted proxies. [Behind a proxy](../advanced/behind-a-proxy.md)

## Lifespan and background tasks

- The deprecated `@app.on_event` hooks have no counterpart, and a resource's `stop` can't report a failure. [Lifespan events](../advanced/events.md)
- `stop` runs only on a graceful shutdown, which `serve_with_shutdown` gives. `serve` runs until the process is killed, where uvicorn shuts down gracefully on Ctrl+C by default. [Server settings](../advanced/server-settings.md#stopping-gracefully)
- A background task added through a copy of `BackgroundTasks` kept after the response is ready is dropped without running. [Background tasks](../tutorial/background-tasks.md)

## The server

- There is no TLS, uvicorn's `--ssl-keyfile`, and no Unix sockets, `--uds`; HTTPS is done by a gateway in front. Like uvicorn, the server speaks only HTTP/1.1. [Server settings](../advanced/server-settings.md)
- A request body is limited to 2 MiB by default, answered with 413, where uvicorn sets no limit; `Server::with_body_limit` changes it.
- A connection that takes more than 30 seconds to send its headers is closed, and so is a keep-alive connection idle that long, where uvicorn closes an idle one after 5 seconds.
- There is no `--workers`, since one process uses every core, nor `--limit-concurrency` or `--reload`.
- The access log goes through `tracing` and stays silent until the program installs a subscriber, where uvicorn's prints by default. Each line also says how long the response took, which uvicorn's doesn't; for a streamed response, up to its head.

## The docs

- The app's `summary`, `description`, `terms_of_service`, `contact` and `license_info`, a description for each tag, and each route's `summary`, `description` and `deprecated` aren't there yet, nor is ReDoc. [Metadata and docs URLs](../tutorial/metadata.md)
- The docs list some answers FastAPI leaves out, read from the types: the 413 of every route reading a body, and the 401 of a route needing credentials.

## Testing

- `TestClient` is async: `TestClient::start(app).await?`, and `client.stop().await` at the end. [Testing](../tutorial/testing.md)
- It has no cookie jar, so a test sends the `Cookie` header itself, and no `base_url`.
