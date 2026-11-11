# Background tasks

A background task runs after the response is sent, so the client doesn't wait for it.
Sending an email after a sign-up is the usual example.
This is FastAPI's `BackgroundTasks`.

This page's app keeps the notifications it sends in a list, so you can see them arrive:

```rust
{{#include ../../../docs_src/src/bin/background_tasks.rs:imports}}

{{#include ../../../docs_src/src/bin/background_tasks.rs:outbox}}
```

## Adding a task

```rust
{{#include ../../../docs_src/src/bin/background_tasks.rs:handler}}
```

A handler that takes `BackgroundTasks` can add tasks to it with `tasks.add`.
A task is any future, here an `async move { ... }` block.
It runs once the response is sent, and tasks run one after another in the order they were added.

`async move` moves what the block uses, `email` and `outbox`, into the task.
The handler has returned by the time the task runs, so the task has to own its values rather than borrow them from the handler.
The compiler checks this: without `move`, it refuses the block, since the borrowed values would be gone before the task used them.

Tasks can also be added from a [dependency](dependencies.md), by listing `BackgroundTasks` in its `type Needs`.
They join the same list as the handler's.

As in FastAPI, a response the handler returns runs its tasks, even one with an error status such as `(StatusCode::NOT_FOUND, ...)`.
An `Err` from the handler doesn't run them, nor does a request refused before the handler, such as a 422.

## State shared by every request

```rust
{{#include ../../../docs_src/src/bin/background_tasks.rs:app}}
```

`.state(value)` gives the app a value that every request can reach.
A handler takes it as `State<Outbox>`, which holds an `Arc<Outbox>`: a pointer shared by everyone holding a copy, counting them, so the value is dropped when the last one goes.
Copying an `Arc` copies only the pointer, so every request sees the same `Outbox`.
A database pool or a client for another service is shared the same way, in place of a module-level global in Python.

A value shared between threads can't be changed by any of them without a type made for it, such as a lock, which Rust checks at compile time.
`Mutex` is a lock: `lock()` gives access to the list until the end of the statement.
A lock is called poisoned when a thread panicked while holding it; `unwrap_or_else(PoisonError::into_inner)` takes the list anyway, since a list of strings can't be left half changed.

`cargo run -p docs_src --bin background_tasks` runs this page's app, and `main` is the same as in [First steps](first-steps.md).

## Seeing it run

`POST /send-notification/foo@example.com` answers at once:

```json
{"message":"Notification sent in the background"}
```

and `/sent` then answers:

```json
["notification for foo@example.com: some notification"]
```

A task that needs a lot of computing, or calls a library that blocks, belongs in `tokio::task::spawn_blocking` inside the task, so it doesn't hold up the other requests on the same thread.
For work that must survive a restart, or run on another machine, a job queue fits better, as FastAPI's docs also say.

## Testing it

```rust
{{#include ../../../docs_src/src/bin/background_tasks.rs:test}}
```

`TestClient` runs the tasks before `send` returns, as FastAPI's does, so the test can check what they did right away.
