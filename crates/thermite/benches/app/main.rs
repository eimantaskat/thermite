// a benchmark that cannot set up has nothing to measure, so setup failures end the run
#![allow(clippy::expect_used)]

mod endpoints;
mod features;
mod support;

use criterion::criterion_main;

criterion_main!(endpoints::benches, features::benches);
