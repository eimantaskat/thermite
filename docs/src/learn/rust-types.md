# Rust types intro

FastAPI reads Python's type hints to know what a request holds, and Thermite reads Rust's types for the same purpose.
In Python a hint is optional and nothing checks it at runtime unless a library like Pydantic does.
In Rust every value has a type, the compiler checks every use of it before the program runs, and a program that mixes them up doesn't build.

This page is the counterpart of FastAPI's "Python types intro": just enough Rust types to read the rest of this guide.

```rust
{{#include ../../../docs_src/src/bin/rust_types.rs:imports}}
```

## Functions

```rust
{{#include ../../../docs_src/src/bin/rust_types.rs:full_name}}
```

A Rust function names a type for each argument and for what it returns, after `->`.
`&str` is text the function borrows, and `String` is text it owns and hands back; [Rust in a handler](rust-in-a-handler.md) explains the difference.
The last expression of a function, without a `;`, is what it returns.
`format!` builds a `String`, and `{first_name}` inside it writes the variable of that name, like a Python f-string.
Only a plain name fits between the braces; an expression such as `self.name` goes after the text, as in `format!("Hello, {}", self.name)` below.

```rust
{{#include ../../../docs_src/src/bin/rust_types.rs:name_with_age}}
```

In FastAPI's version, `name + " is this old: " + age` fails at runtime, since a `str` can't be added to an `int`.
In Rust the same mistake doesn't compile: `+` on a `String` takes only text.
`format!` writes any value that implements `Display`, the trait for writing a value as text, which numbers do.

## Simple types

| Python | Rust | Notes |
| --- | --- | --- |
| `int` | `i64` | Rust also has `i32`, `u8`, `u64` and others; Thermite's models take `i64` |
| `float` | `f64` | |
| `bool` | `bool` | `true` and `false`, in lowercase |
| `str` | `String`, or `&str` when borrowed | Always UTF-8 |
| `bytes` | `Vec<u8>`, or `Bytes` for a body | |
| `None` | `Option<T>`, holding `None` or `Some(value)` | See below |

A number doesn't turn into another type by itself: `i64` to `f64` is written `number as f64`, and text to a number is `text.parse::<i64>()`, which can fail and returns a `Result`.

## Generic types

```rust
{{#include ../../../docs_src/src/bin/rust_types.rs:generics}}
```

A type with another inside it is written with angle brackets: `Vec<String>` is Python's `list[str]`.
`&[String]` is a borrowed run of strings, which a `Vec<String>` or an array can lend; a function that only reads a list takes it this way.
`(i64, String)` is a tuple, `tuple[int, str]`, and `let (number, text) = pair;` unpacks it as Python does.

`.iter()` walks the items, `.map(...)` changes each, and `.collect()` gathers them into the type the function returns.
`|item| item.to_uppercase()` is a closure, a small function written inline, like `lambda item: item.upper()`.

| Python | Rust |
| --- | --- |
| `list[str]` | `Vec<String>` |
| `tuple[int, str]` | `(i64, String)` |
| `set[str]` | `HashSet<String>` |
| `dict[str, float]` | `HashMap<String, f64>` |

Sets and maps are in `std::collections`, and they have no schema in Thermite yet, so a model can't hold one; a handler can still use them inside.

## Optional values

```rust
{{#include ../../../docs_src/src/bin/rust_types.rs:option}}
```

Rust has no `None` that any variable can hold.
A value that may be missing is an `Option<T>`, either `Some(value)` or `None`, Python's `str | None`.
The compiler won't let the code use the value inside before checking which one it is, so a missing value can't surprise the program at runtime.

`match` takes each case in turn, like Python's `match` statement, and the compiler checks that every case is covered.
`if let Some(name) = name { ... }` handles the one case when the other has nothing to do.

## Structs

```rust
{{#include ../../../docs_src/src/bin/rust_types.rs:person}}
```

A `struct` is the counterpart of a class's fields, and the type of anything built from it.
Its methods go in an `impl` block, and `&self` is Python's `self`, borrowed.
There is no inheritance; [traits](rust-in-a-handler.md#traits) are how types share behaviour.

## Models

FastAPI's models are Pydantic classes.
In Thermite they are structs that derive three traits:

```rust
{{#include ../../../docs_src/src/bin/rust_types.rs:model}}
```

- `Deserialize`, from [serde](https://serde.rs/), reads a `User` from JSON;
- `Serialize`, from serde too, writes one as JSON;
- `Model`, from Thermite, describes its fields as a schema, which validates the request and fills the docs.

A model only reading requests needs `Deserialize` and `Model`, and one only answering needs `Serialize` and `Model`.
`Option<String>` makes a field optional, and `#[model(default = "John Doe")]` gives a missing field a value, Pydantic's `name: str = "John Doe"`.
`#[model(default)]` without a value gives an empty list, Pydantic's `friends: list[int] = []`.

As in Pydantic, a value that is close enough is converted: a request to `POST /users` with

```json
{"id": "123", "signup_ts": "2017-06-01 12:22", "friends": [1, "2", 3]}
```

answers

```json
{"id":123,"name":"John Doe","signup_ts":"2017-06-01 12:22","friends":[1,2,3]}
```

The text `"123"` became the number `123`, and one that can't be read as a number answers 422, as [Request body](../tutorial/body.md) shows.
Dates have no type yet, so `signup_ts` stays text.

```rust
{{#include ../../../docs_src/src/bin/rust_types.rs:app}}
```

`cargo run -p docs_src --bin rust_types` runs this page's app, and [First steps](../tutorial/first-steps.md) explains `App`, `routes!` and `main` line by line.

## Where FastAPI uses `Annotated`

FastAPI says where a parameter comes from with `Annotated[str, Header()]` or a default such as `Query()`.
Rust has no annotations on a type, so Thermite wraps it instead: `Path<T>`, `Query<T>`, `Header<T>`, `Cookie<T>`, `Json<T>` and `Form<T>` each read a model from their part of the request.
The handler names the wrapper as an argument's type, and [Path parameters](../tutorial/path-params.md) starts with the first of them.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/rust_types.rs:test}}
```

`#[test]` marks a test that doesn't need a runtime, and `#[tokio::test]` one that does, as every test sending requests does.
