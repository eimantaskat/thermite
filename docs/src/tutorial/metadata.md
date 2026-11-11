# Metadata and docs URLs

The app's title and version, and the paths of its docs, are set on the `App`:

```rust
{{#include ../../../docs_src/src/bin/metadata.rs:imports}}

{{#include ../../../docs_src/src/bin/metadata.rs:app}}
```

`cargo run -p docs_src --bin metadata` runs this page's app, and `main` is the same as in [First steps](first-steps.md).

## Title and version

The schema at `/api/v1/openapi.json` starts with:

```json
"info": {"title": "ChimichangApp", "version": "0.0.1"}
```

The title is also the title of the docs page, `ChimichangApp - Swagger UI`.
Without `.title` and `.version`, they are `Thermite` and `0.1.0`.

## Tags

The tags come from the routers, as on [Bigger applications](bigger-applications.md): `GET /users` has `"tags": ["users"]`, and Swagger UI shows the routes grouped under each tag.

## Docs URLs

`.openapi_path` moves the schema, and `.docs_path` moves Swagger UI, which reads the schema from its new path.
Here Swagger UI is at `/documentation`, and `/docs` is a 404.

`.without_docs()` turns Swagger UI off and keeps the schema, as `docs_url=None` does in FastAPI.
`.without_openapi()` turns off both, as `openapi_url=None` does, since the docs page has nothing to read without the schema.

## What is missing

FastAPI also takes a `summary`, a `description` in Markdown, `terms_of_service`, `contact` and `license_info` for the app, a description for each tag, and a `summary`, `description` and `deprecated` for each route.
Thermite doesn't have those yet, so the docs show only what the code says.
There is no ReDoc page either, only Swagger UI.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/metadata.rs:test}}
```
