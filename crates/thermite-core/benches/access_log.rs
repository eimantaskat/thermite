// a binary of its own: tracing caches whether a log call is wanted once a subscriber is
// registered, so the other benchmarks would no longer run as an app without one does
// a benchmark that cannot set up has nothing to measure, so setup failures end the run
#![allow(clippy::expect_used)]

#[path = "core/http_client.rs"]
mod http_client;

use std::fmt::{Debug, Write as _};
use std::hint::black_box;
use std::io::{Write as _, sink};
use std::time::Instant;

use bytes::Bytes;
use criterion::{Criterion, criterion_group, criterion_main};
use http::{Method, StatusCode};
use thermite_core::{Request, Response, Server};
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Dispatch, Event, Metadata, Subscriber};

use http_client::{connect, send};

const ACCESS_TARGET: &str = "thermite::access";

// wants every access event, then throws it away: what building the event costs
struct Dropping;

// writes each event as a line, as tracing-subscriber's fmt layer would, into a sink
struct Formatting;

struct Line<'line>(&'line mut String);

impl Visit for Line<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let _ = write!(self.0, " {}={value:?}", field.name());
    }
}

macro_rules! access_only_subscriber {
    ($subscriber:ty, |$event:ident| $body:expr) => {
        impl Subscriber for $subscriber {
            fn enabled(&self, metadata: &Metadata<'_>) -> bool {
                metadata.target() == ACCESS_TARGET
            }

            fn new_span(&self, _: &Attributes<'_>) -> Id {
                Id::from_u64(1)
            }

            fn record(&self, _: &Id, _: &Record<'_>) {}

            fn record_follows_from(&self, _: &Id, _: &Id) {}

            fn event(&self, $event: &Event<'_>) {
                $body
            }

            fn enter(&self, _: &Id) {}

            fn exit(&self, _: &Id) {}
        }
    };
}

access_only_subscriber!(Dropping, |event| {
    black_box(event);
});

access_only_subscriber!(Formatting, |event| {
    let mut line = format!("{} {}", event.metadata().level(), event.metadata().target());
    event.record(&mut Line(&mut line));
    let _ = writeln!(sink(), "{line}");
});

fn access_log(criterion: &mut Criterion) {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let address = runtime.block_on(async {
        let server = Server::bind("127.0.0.1:0").await.expect("a free port");
        let address = server.local_address().expect("the bound address");
        tokio::spawn(server.serve(|_: Request| async { Response::new(Bytes::from_static(b"ok")) }));
        address
    });
    let empty = Bytes::new();

    let mut group = criterion.benchmark_group("access_log");
    for (name, dispatch) in [
        ("dropped", Dispatch::new(Dropping)),
        ("formatted", Dispatch::new(Formatting)),
    ] {
        // the server's tasks run on this thread, inside the runtime's block_on
        tracing::dispatcher::with_default(&dispatch, || {
            let status = runtime.block_on(async {
                send(&mut connect(address).await, Method::GET, "/", &empty).await
            });
            assert_eq!(status, StatusCode::OK);
            group.bench_function(name, |bencher| {
                bencher.to_async(&runtime).iter_custom(|iterations| {
                    let empty = &empty;
                    async move {
                        let mut sender = connect(address).await;
                        let started = Instant::now();
                        for _ in 0..iterations {
                            black_box(send(&mut sender, Method::GET, "/", empty).await);
                        }
                        started.elapsed()
                    }
                });
            });
        });
    }
    group.finish();
}

criterion_group!(benches, access_log);
criterion_main!(benches);
