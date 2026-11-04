use bytes::Bytes;
use criterion::measurement::WallTime;
use criterion::{BatchSize, BenchmarkGroup};
use http::StatusCode;
use thermite_core::{Arguments, Request, Response, Router};
use tokio::runtime::{Builder, Runtime};

// one thread, so a run measures the request path rather than the scheduler
pub fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime")
}

pub fn request(builder: http::request::Builder, body: impl Into<Bytes>) -> Request {
    builder.body(body.into()).expect("a valid request")
}

pub async fn answer_ok(_: Request, _: Arguments) -> Response {
    Response::new(Bytes::from_static(b"ok"))
}

pub fn router(descriptions: impl IntoIterator<Item = thermite_core::RouteDescription>) -> Router {
    let mut router = Router::new();
    for description in descriptions {
        router
            .route(description, answer_ok)
            .expect("a route that registers");
    }
    router
}

// requests are built outside the measured time, as a real server gets them from hyper; the status
// is checked first, so a typo in a benchmark cannot quietly measure a 404 or a 422
pub fn bench_handle(
    group: &mut BenchmarkGroup<'_, WallTime>,
    runtime: &Runtime,
    name: &str,
    router: &Router,
    expected: StatusCode,
    request: impl Fn() -> Request,
) {
    let response = runtime.block_on(router.handle(request()));
    assert_eq!(response.status(), expected, "{name}");
    group.bench_function(name, |bencher| {
        bencher.to_async(runtime).iter_batched(
            &request,
            |request| router.handle(request),
            BatchSize::SmallInput,
        );
    });
}
