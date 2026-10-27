# thermite

[![CI](https://github.com/eimantaskat/thermite/actions/workflows/ci.yml/badge.svg)](https://github.com/eimantaskat/thermite/actions/workflows/ci.yml)

A FastAPI-style web framework with a Rust core, usable from Rust and, later, from Python.

## Status

An experimental project in an early state. Not ready for real use, and the API will change.

## Layout

| Crate | Purpose |
| --- | --- |
| `thermite-core` | The runtime: server, router, and later validation, errors and OpenAPI, all driven by route descriptions as plain data |
| `thermite` | Rust front-end that turns Rust handlers into route descriptions |
| `thermite-py` | Python front-end built with PyO3 (planned) |

## Development

The toolchain is pinned in `rust-toolchain.toml`, so `rustup` installs the right version on first use.

```sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
cargo llvm-cov --workspace --all-features --summary-only   # needs cargo-llvm-cov
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
