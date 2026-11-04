// a benchmark that cannot set up has nothing to measure, so setup failures end the run
#![allow(clippy::expect_used)]

mod extraction;
mod http_client;
mod routing;
mod server;
mod support;
mod validation;

use criterion::criterion_main;

criterion_main!(
    routing::benches,
    extraction::benches,
    validation::benches,
    server::benches
);
