# Static files

`StaticFiles` serves the files of a directory, as FastAPI's `StaticFiles` does:

```rust
{{#include ../../../docs_src/src/bin/static_files.rs:imports}}

{{#include ../../../docs_src/src/bin/static_files.rs:app}}
```

`mount("/static", ...)` serves the directory `static` under `/static`, so `static/hello.txt` is at `/static/hello.txt`.
The directory is relative to where the program runs, so this page's app is run from the `docs_src` directory, which holds `static`:

```console
$ cd docs_src
$ cargo run --bin static_files
```

`/static/hello.txt` answers `Hello from a file`, with `Content-Type: text/plain; charset=utf-8` guessed from the extension.
A file that doesn't exist is the usual 404:

```json
{"detail":"Not Found"}
```

The answers carry `ETag` and `Last-Modified`, so a browser can ask whether its copy is still current and get a 304 without the file.
Range requests, which a video player or a resumed download sends, are answered with only the part asked for: `Range: bytes=0-4` answers 206 with `Hello`.

A directory that doesn't exist is an error from `mount`, so the server doesn't start without its files:

```text
the static files mounted at `/static` come from `no-such-directory`, which is not a directory
```

`.check_dir(false)` turns that check off, for a directory created after the app is built.

## Routes and mounts

Routes are tried before mounts, wherever the mount was added.
This differs from FastAPI, where routes and mounts are tried in the order they were added, so a mount at `/` has to come last.
It allows a mount at `/` with `.html(true)`, which serves a directory's `index.html` and a missing file's `404.html`, so a built front-end can sit behind the API routes:

```rust
{{#include ../../../docs_src/src/bin/static_files.rs:single_page}}
```

Here `/` still answers `Hello World` from the route, while `/hello.txt` and `/index.html` come from the directory.
A path that is neither is a 404, not `index.html`, so a single-page app whose own router handles paths such as `/settings` still needs a route answering those with `index.html`.

A `Router` can mount files too, under its prefix.
Mounts don't appear in the docs, as in FastAPI.

## Behind a gateway

Files are often served by the gateway or a CDN in front of the app instead, which is faster at it.
Then the app needs no mount.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/static_files.rs:test}}
```

`cargo test` runs a crate's tests from the crate's directory, so `static` is found there.
