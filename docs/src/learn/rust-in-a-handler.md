# Rust in a handler

A few ideas of Rust have no counterpart in Python, and a handler meets them from its first lines: who owns a value, traits, and errors as values.
This page goes through them on a small app that stores items in memory.

```rust
{{#include ../../../docs_src/src/bin/rust_in_a_handler.rs:imports}}
```

## Ownership

```rust
{{#include ../../../docs_src/src/bin/rust_in_a_handler.rs:handlers}}
```

In Python, every variable is a reference to an object, and the object lives while anything refers to it.
In Rust, each value has one owner, and it is freed when its owner goes away, with no garbage collector.

`create_item` owns the `item` read from the body.
`store.add(item)` moves it into the store, which owns it from then on, and the handler can't use `item` after that line: the compiler refuses `item.name` below it, saying the value was moved.
That is why the message is written first, while the handler still owns the item.

`.clone()` makes a second copy; for `Item`, whose fields are a `String` and a number, it is a deep copy, as `copy.deepcopy` would make.
What a clone copies depends on the type: cloning an `Arc`, or a `State` holding one, copies only the pointer.
`find` hands out an item this way while the store keeps its own: `.cloned()` turns the `Option<&Item>` that `get` returns into an `Option<Item>` by cloning it, which is why `Item` derives `Clone`.
A clone of a large value costs time, so it is written out where it happens, never done silently.

### Borrowing

Most of the time a function only needs to look at a value, so it borrows it instead of taking it.
`&name` lends the handler's `String` to `find`, which reads it and gives it back by returning, so the handler still owns `name` afterwards.

A borrow is a reference the compiler checks: it can't outlive the value it borrows, and while something holds `&mut`, a borrow that can change the value, nothing else can use it.
This is why a [background task](../tutorial/background-tasks.md) or a [stream](../advanced/stream-data.md) takes its values with `move`: it runs after the handler has returned, so it has to own what it uses.

`&str` is borrowed text, and a function that only reads text takes `&str`, which a `String` or a literal such as `"Foo"` can lend.
A struct field that keeps the text is a `String`, which `.to_owned()` makes from a `&str`.

### Shared state

```rust
{{#include ../../../docs_src/src/bin/rust_in_a_handler.rs:store}}
```

Every request reaches the same `Store` through `State<Store>`, which holds an `Arc`, a pointer every request shares.
A value shared that way can only be borrowed with `&`, so nothing could change it, unless a type made for it, here a `Mutex`, lets one thread at a time in.
`lock()` gives `&mut` access to the map until `items` goes away at the end of the method, as [Background tasks](../tutorial/background-tasks.md#state-shared-by-every-request) explains.

`items.insert(item.name.clone(), item)` clones the name for the key, since the map owns its keys and the item owns its name.

## Traits

A trait is a set of methods a type promises to have, close to a Python `Protocol` or an abstract base class.
A type implements a trait in an `impl Trait for Type` block, which may be written anywhere in the crate, including for a type from another crate, as long as one of the crate's own types appears in it.
In `impl From<StoreError> for HttpError` below, neither `From` nor `HttpError` is this crate's, but `StoreError` is.

```rust
{{#include ../../../docs_src/src/bin/rust_in_a_handler.rs:error}}
```

- `Display` is how a value is written as text, Python's `__str__`; `to_string()` comes with it.
- `Error` marks the type as an error; it needs `Debug` and `Display`, and its own methods all have defaults, so its `impl` is empty.
- `From<StoreError> for HttpError` says how a store error becomes an HTTP error, here with a 404 or a 409.

Thermite is built on traits as well, and their names turn up in compiler messages:

- `FromRequest`: what a handler can take as an argument, such as `Path<T>` or `State<T>`;
- `IntoResponse`: what a handler can return, such as `Json<T>`, a `String` or a `Result` of two of them;
- `Dependency`: what `Depends<T>` can resolve, as on [Dependencies](../tutorial/dependencies.md);
- `Model`, `Serialize` and `Deserialize`: what a model derives, as on [Rust types intro](rust-types.md#models).

`#[derive(...)]` asks the compiler to write a trait's implementation from the type's fields, as with `Clone` above.

`enum StoreError` is a type with a fixed set of cases, each able to hold its own data, here the item's name.
`match` on it must name every case, so adding a third one makes the compiler point at each `match` left to update.

## Errors as values

Rust has no exceptions.
A function that can fail returns a `Result<T, E>`, either `Ok(value)` or `Err(error)`, and the caller has to deal with both before it can use the value.

`?` after a `Result` gives the value inside an `Ok`, and returns early with the error from an `Err`, the way an exception leaves a Python function.
On the way, it converts the error with `From` into the type the function returns: `store.add(item)?` returns a `StoreError` from `add`, and the handler, which returns `HttpError`, answers with the status the `From` above picks.
`POST /items` with an item that already exists answers 409:

```json
{"detail":"an item named Foo already exists"}
```

and `/items/Bar` answers 404:

```json
{"detail":"no item named Bar"}
```

`Option` has the same shape: `ok_or_else(|| ...)` turns `None` into an error, so `?` can return it.

`unwrap()` takes the value out of a `Result` or an `Option` and panics when there is none, which the server answers with a 500, and which fails a test unless its `TestClient` catches panics.
It fits a test, where a panic is a failure to report, and Thermite's own code never uses it outside them.
In an app, `?` or a `match` says what happens instead.

[Handling errors](../tutorial/handling-errors.md) shows `HttpError` and exception handlers in full.

## The app

```rust
{{#include ../../../docs_src/src/bin/rust_in_a_handler.rs:app}}
```

`cargo run -p docs_src --bin rust_in_a_handler` runs this page's app, and `main` is the same as in [First steps](../tutorial/first-steps.md).

## Testing it

```rust
{{#include ../../../docs_src/src/bin/rust_in_a_handler.rs:test}}
```
