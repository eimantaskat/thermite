# Concurrency and async

FastAPI's "Concurrency and async / await" page explains when to write `async def` and when plain `def`.
In Thermite every handler is an `async fn`, and what matters instead is never to block inside one.
This page explains why, and the two rules the compiler adds: what crosses threads must be `Send`, and a lock must not be held across an `.await`.

```rust
{{#include ../../../docs_src/src/bin/concurrency.rs:imports}}
```

## async and await

```rust
{{#include ../../../docs_src/src/bin/concurrency.rs:order}}
```

As in Python, calling an `async fn` doesn't run it: it gives a future, Rust's coroutine, which runs when something awaits it.
`.await` is written after the value rather than before it, so calls chain, as in `client.get("/").send().await?`.
While a future waits, here on a timer, the thread runs other futures, such as other requests.

`tokio::join!` awaits several futures at once and gives all their results, like `asyncio.gather`.
`/order` waits for both timers together, so it takes about 100 ms rather than 200 ms, and answers:

```json
{"burgers":"2 burgers","drinks":"2 drinks"}
```

`tokio::time::sleep` is `asyncio.sleep`, and needs tokio's `time` feature.
`std::thread::sleep` would be `time.sleep`, which blocks the whole thread.

## Threads and the runtime

Python's asyncio runs one event loop on one thread, and uvicorn uses several cores only when `--workers` starts several processes.
[tokio](https://tokio.rs/), the runtime `#[tokio::main]` starts, runs a worker thread for each core, and any of them can run any request's future.
So one process uses every core, and two requests can run their handlers at the same moment.

FastAPI runs a plain `def` handler in a thread pool so that its blocking calls don't stop the event loop.
Thermite has no such handler: an `async fn` runs on a worker thread, and a blocking call in it, such as `std::fs::read`, a synchronous database driver or a long computation, holds up that thread and every request waiting on it.
Async libraries, such as `tokio::fs` or an async database driver, wait without blocking.

## Blocking work

```rust
{{#include ../../../docs_src/src/bin/concurrency.rs:blocking}}
```

`tokio::task::spawn_blocking` runs a plain function on a separate pool of threads kept for blocking work, the counterpart of FastAPI's thread pool, and its future gives the result when it is done.
`/primes?limit=100` answers:

```json
{"limit":100,"count":25}
```

`move` gives the closure its own copy of `limit`, since it runs on another thread and may outlive the handler.
The `?` after `.await` is for the task itself, which fails if the function panicked or the runtime shut down before it ran, and `thermite::Error` takes that error as it takes [your own errors](../tutorial/handling-errors.md#your-own-errors), answering 500.

A limit over ten million answers 422 with `{"detail":"limit must be at most 10000000"}`, since one request could otherwise keep a blocking thread busy for minutes; a check that a model can't express goes in the handler, as on [Request body](../tutorial/body.md#checks-a-model-cant-express-yet).

A task of a few microseconds isn't worth sending away, but one that may take milliseconds is, such as hashing a password or resizing an image.

## Send

A future may start on one thread and carry on, after an `.await`, on another.
So Thermite asks that every handler's future is `Send`: everything it holds across an `.await` must be safe to move to another thread.
Most types are; an `Rc`, a `&RefCell`, or a guard of a `std::sync::Mutex` are not.

`Sync` is the related promise, that a value can be used through `&` from several threads at once, which is what [state](../tutorial/background-tasks.md#state-shared-by-every-request) needs, since every request shares it: `State<T>` asks `T` to be `Send + Sync`.
`Mutex<T>` is `Sync` whenever `T` is `Send`, which is why shared values that change sit in one.

The compiler checks both, so a value that isn't safe to share is a build error, never a data race at runtime, where two threads change the same memory at once.

## Locks and await

```rust
{{#include ../../../docs_src/src/bin/concurrency.rs:lock}}
```

The guard that `lock()` returns keeps the lock until it goes away.
Here it lives in a block of its own, `{ ... }`, which ends before the `.await`, so the lock is free while the handler waits, and the block's last expression, `*count`, is the number it gives back.
`*count` reads and writes the `i64` behind the guard.

Kept until the end of the function instead, the guard would be held across the `.await`.
Every other request for `/visit` would then wait for the lock while this one sleeps, and the guard isn't `Send`, so the compiler refuses the handler.
`drop(count)` before the `.await` doesn't satisfy the compiler either, which still counts the guard as held; only a block that ends first does.
The message says the function "cannot handle a route" and is "not a handler", without naming the guard, so when a handler that looks right is refused, a guard or another value that isn't `Send` living across an `.await` is the first thing to look for.

When a lock truly has to be held across an `.await`, `tokio::sync::Mutex` allows it, at the cost of a slower lock; most code keeps the lock short instead.

## The app

```rust
{{#include ../../../docs_src/src/bin/concurrency.rs:app}}
```

`cargo run -p docs_src --bin concurrency` runs this page's app, and `main` is the same as in [First steps](../tutorial/first-steps.md).

## Testing it

```rust
{{#include ../../../docs_src/src/bin/concurrency.rs:test}}
```
