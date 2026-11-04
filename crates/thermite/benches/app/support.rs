use criterion::measurement::WallTime;
use criterion::{BatchSize, BenchmarkGroup};
use thermite::{App, Bytes, Request, StatusCode};
use thermite_core::InProcessServer;
use tokio::runtime::{Builder, Runtime};

// one thread, so a run measures the request path rather than the scheduler
pub fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime")
}

// the server the TestClient wraps, sent requests built beforehand so building them is not measured
pub fn start(runtime: &Runtime, app: App) -> InProcessServer<App> {
    runtime
        .block_on(InProcessServer::start(app))
        .expect("an app that starts")
}

pub fn request(builder: thermite::http::request::Builder, body: impl Into<Bytes>) -> Request {
    builder.body(body.into()).expect("a valid request")
}

// the status is checked first, so a typo in a benchmark cannot quietly measure a 404 or a 422
pub fn bench_send(
    group: &mut BenchmarkGroup<'_, WallTime>,
    runtime: &Runtime,
    name: &str,
    server: &InProcessServer<App>,
    expected: StatusCode,
    request: impl Fn() -> Request,
) {
    let response = runtime.block_on(server.send(request()));
    assert_eq!(response.status(), expected, "{name}");
    group.bench_function(name, |bencher| {
        bencher.to_async(runtime).iter_batched(
            &request,
            |request| server.send(request),
            BatchSize::SmallInput,
        );
    });
}
