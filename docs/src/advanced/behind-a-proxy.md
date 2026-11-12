# Behind a proxy

In production an app usually sits behind a gateway or reverse proxy, such as nginx, Traefik, Kong or a cloud load balancer.
The gateway takes the HTTPS connection, routes requests by path and passes them on to the app over plain HTTP.
This page covers what that changes for the app.

```rust
{{#include ../../../docs_src/src/bin/behind_a_proxy.rs:imports}}
```

## A path prefix

FastAPI's `root_path` serves an app under a prefix that the gateway strips: the gateway gets `/api/v1/items` and sends `/items` to the app, and `root_path="/api/v1"` tells the docs where they really are.
Thermite has no `root_path` yet.
Instead, the gateway passes the path on as it is, and the app serves everything under the prefix itself:

```rust
{{#include ../../../docs_src/src/bin/behind_a_proxy.rs:app}}
```

The routes go in a [router](../tutorial/bigger-applications.md) with the prefix, and the docs and the schema move under it with `docs_path` and `openapi_path`, as on [Metadata and docs URLs](../tutorial/metadata.md).
The docs then list `/api/v1/client`, Swagger UI at `/api/v1/docs` loads `/api/v1/openapi.json`, and "Try it out" sends its requests through the gateway to the right path.
A request to `/client` without the prefix answers 404.

In nginx, `proxy_pass http://app:8000;` without a path after the address passes the path unchanged, and so does Traefik unless a `StripPrefix` middleware is added.
Kong strips the matched path by default, so its route needs `strip_path: false`.

## The client's address and scheme

The app sees the gateway as its client, so the gateway sends the original client's address and scheme in headers, most often `X-Forwarded-For` and `X-Forwarded-Proto`.
They are read like any other [header](../tutorial/header-cookie-params.md).

`X-Forwarded-For` lists every proxy a request went through, and each one appends the address it saw.
Anything before the last address came from the client, who can send the header with any value, so only the last one, added by your gateway, can be trusted.
`rsplit(',').next()` takes that one: `rsplit` splits from the end, and `next()` gives its first part.
With two gateways in a row, the trusted address is the second to last, and so on.

With `X-Forwarded-For: 203.0.113.7, 198.51.100.2` and `X-Forwarded-Proto: https`, `/api/v1/client` answers:

```json
{"client":"198.51.100.2","scheme":"https"}
```

Uvicorn reads these headers itself with `--proxy-headers` and `--forwarded-allow-ips`, setting `request.client` and the URL's scheme.
Thermite doesn't read them yet, and doesn't give the app the address of the connection either, so the headers are the only way to know the client.
An app reachable without the gateway should not trust them at all.

## HTTPS

The gateway holds the certificate and talks HTTPS with clients, and the app speaks plain HTTP to the gateway, which suits Thermite, since its server has no TLS.
The connection between them is inside your network, or on the same machine.

The redirects Thermite sends, such as the trailing-slash redirect of [static files](../tutorial/static-files.md), give a path without a scheme or host, so the browser stays on `https://` and the gateway's host.
Starlette writes a whole URL there, which is why a FastAPI app behind a gateway needs the forwarded scheme to redirect to `https://`.

## The server behind a gateway

```rust
{{#include ../../../docs_src/src/bin/behind_a_proxy.rs:main}}
```

- `0.0.0.0` listens on every network interface, so a gateway in another container or machine can reach the app; `127.0.0.1` accepts only connections from the same machine.
- `with_header_read_timeout(None)` turns off the header read timeout, which also closes idle keep-alive connections, so the gateway's pool of connections to the app stays open as long as the gateway wants; the gateway already protects the app from slow clients.
- The [body limit](server-settings.md) still applies, and the gateway's own limit, such as nginx's `client_max_body_size` of 1 MiB by default, has to be at least as large.
- [CORS](../tutorial/cors.md) and [GZip](../tutorial/middleware.md#gzip) are often done by the gateway, and then stay off in the app.

`cargo run -p docs_src --bin behind_a_proxy` runs this page's app on port 8000 of every interface.

## What is missing

`root_path`, the `servers` list in the docs, reading the forwarded headers with a list of trusted proxies, the standard `Forwarded` header, and the connection's own address are not in Thermite yet.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/behind_a_proxy.rs:test}}
```

A test sends the headers a gateway would, and a request without them gets the app's defaults.
