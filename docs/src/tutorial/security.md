# Security

This page builds FastAPI's "Simple OAuth2 with Password and Bearer": a client sends a username and password to `/token`, gets a token back, and sends the token with each request in an `Authorization: Bearer ...` header.

## The scheme

```rust
{{#include ../../../docs_src/src/bin/security.rs:imports}}

{{#include ../../../docs_src/src/bin/security.rs:scheme}}
```

`#[derive(SecurityScheme)]` with `oauth2_password` is FastAPI's `OAuth2PasswordBearer(tokenUrl="token")`.
`Token` holds the token from the `Authorization` header, and a handler or dependency that takes a `Token` gets it.

A request without the header is answered with a 401, and the handler is not called.
The answer has `WWW-Authenticate: Bearer` and this body:

```json
{"detail":"Not authenticated"}
```

The docs list the scheme:

```json
"securitySchemes": {
  "OAuth2PasswordBearer": {
    "type": "oauth2",
    "flows": {"password": {"scopes": {}, "tokenUrl": "token"}}
  }
}
```

Every route that needs a `Token` is marked with `"security": [{"OAuth2PasswordBearer": []}]`, so Swagger UI shows a lock next to it.
The "Authorize" button at the top of `/docs` asks for a username and password, sends them to `/token`, and adds the token to each request it makes from then on.

The other schemes are written the same way:

- `#[security(api_key, header = "x-key")]`, or `query = ...`, or `cookie = ...`, holding a `String`;
- `#[security(http_basic)]`, holding an `HttpBasicCredentials` with `username` and `password`;
- `#[security(http_bearer)]`, holding an `HttpAuthorizationCredentials`.

Taking `Option<Token>` in place of `Token` makes the credentials optional, as `auto_error=False` does in FastAPI: a request without them gets `None` instead of a 401.

## The users

This example keeps its users in a list:

```rust
{{#include ../../../docs_src/src/bin/security.rs:users}}
```

`static` declares a value that lives for the whole run of the program.
`&'static str` is a borrowed string that lives that long too: string literals are stored in the program itself, so they always are.
`find_user` hands back a `&'static StoredUser`, a reference into the list that never stops being valid, so the caller can keep it as long as it likes.

## Logging in

```rust
{{#include ../../../docs_src/src/bin/security.rs:login}}
```

The username and password come as a form, as OAuth2 requires; [Form data](request-forms.md) covers forms.
`Some(user) if ...` is a pattern with a guard: it matches only when a user was found and the guard is true, and anything else falls through to `_`.

Sending `username=johndoe` and `password=secret` answers:

```json
{"access_token":"johndoe","token_type":"bearer"}
```

A wrong password answers 400:

```json
{"detail":"Incorrect username or password"}
```

FastAPI reads this form with `OAuth2PasswordRequestForm`, which Thermite doesn't have yet.
A `Form` model with `username` and `password` does the same here, and the other fields Swagger UI sends, such as `grant_type`, are ignored.

As in FastAPI's example, the token is just the username, so anyone can make one.
A real app hashes the stored passwords and signs its tokens, for example as JWTs.
Crates such as `argon2` and `jsonwebtoken` do that work.

## The current user

The token turns into a user in a dependency, and a second dependency refuses disabled users:

```rust
{{#include ../../../docs_src/src/bin/security.rs:current_user}}
```

`let ... else` unpacks a value or leaves the function: `let Some(stored) = ... else { return ... }` binds `stored` when there is a user and returns the error when there isn't.

With `Authorization: Bearer johndoe`, `/users/me` answers:

```json
{"username":"johndoe","full_name":"John Doe"}
```

A token that names nobody answers 401 with `WWW-Authenticate: Bearer`:

```json
{"detail":"Invalid authentication credentials"}
```

and `Bearer alice`, a disabled user, answers 400:

```json
{"detail":"Inactive user"}
```

`HttpError::with_header` adds the `WWW-Authenticate` header that OAuth2 asks for, as `headers={"WWW-Authenticate": "Bearer"}` does on FastAPI's `HTTPException`.

The routes are listed in `app()`, and `main` is the same as in [First steps](first-steps.md):

```rust
{{#include ../../../docs_src/src/bin/security.rs:app}}
```

`cargo run -p docs_src --bin security` runs this page's app.

## What is missing

Thermite has no OAuth2 scopes yet, FastAPI's `SecurityScopes`, and no OpenID Connect scheme.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/security.rs:test}}
```
