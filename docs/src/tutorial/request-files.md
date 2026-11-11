# Request files

A form with a file input sends a `multipart/form-data` body.
An `UploadFile` field in a `Form` model reads a file from it, as FastAPI's `UploadFile` does:

```rust
{{#include ../../../docs_src/src/bin/request_files.rs:imports}}

{{#include ../../../docs_src/src/bin/request_files.rs:upload}}
```

An `UploadFile` gives the file's `filename()`, its `content_type()` if the client sent one, its `size()` in bytes, and its `content()`.
Uploading `notes.txt`, holding `hello`, answers:

```json
{"filename":"notes.txt","content_type":"text/plain","size":5}
```

`size()` is a `usize`, Rust's type for sizes and indexes, while a field in a response model is an `i64` (the other integer types aren't supported yet).
`i64::try_from` converts it, failing for a size too large to fit, which can't happen for a file held in memory, so `unwrap_or(i64::MAX)` fills that case in.

`content_type()` returns an `Option<&str>`, borrowed from the file, and `.map(str::to_owned)` turns it into an `Option<String>` the response owns.

The docs write the field as a file:

```json
"file": {
  "type": "string",
  "contentMediaType": "application/octet-stream",
  "title": "File"
}
```

and list the body under `multipart/form-data`:

```json
"multipart/form-data": {
  "schema": {"$ref": "#/components/schemas/Upload"}
}
```

FastAPI lists a form model with files under `application/x-www-form-urlencoded`, which no client could send a file in.

## Text fields, many files, and no file

A model can mix text fields with files, hold a list of files, or make a file optional:

```rust
{{#include ../../../docs_src/src/bin/request_files.rs:many}}
```

Sending `title=June` with two `attachments` answers:

```json
{"title":"June","attachments":[{"filename":"a.csv","content_type":"text/csv","size":3},{"filename":"b.csv","content_type":"text/csv","size":5}],"cover":null}
```

and `title=July` alone answers:

```json
{"title":"July","attachments":[],"cover":null}
```

`report.attachments.iter().map(Uploaded::of).collect()` goes over the files, turns each into an `Uploaded`, and gathers them in a `Vec`, which `collect` knows to build from the field's type.
`.as_ref()` on the `Option` borrows the file inside, since `Uploaded::of` takes a reference, `&UploadFile`.

The routes are listed in `app()`, and `main` is the same as in [First steps](first-steps.md):

```rust
{{#include ../../../docs_src/src/bin/request_files.rs:app}}
```

`cargo run -p docs_src --bin request_files` runs this page's app.

## Size limit

Files are held in memory, so the server's body limit, 2 MiB unless set, caps an upload.
A larger body is answered with a 413.
`Server::with_body_limit` raises the limit.

FastAPI's `UploadFile` keeps a large file on disk instead; Thermite doesn't do that yet.
FastAPI also takes `bytes` for a file's content; here the content is `UploadFile::content`.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/request_files.rs:test}}
```

`TestMultipart` builds the body: `.file(name, filename, content type, content)` adds a file, and `.text(name, value)` a text field.
