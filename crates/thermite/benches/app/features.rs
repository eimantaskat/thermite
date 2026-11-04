// dependencies are written here the way users write them, as async fns that may not await
#![allow(clippy::unused_async_trait_impl)]

use std::convert::Infallible;
use std::num::ParseIntError;

use criterion::{Criterion, criterion_group};
use thermite::{
    App, BackgroundTasks, Dependency, Depends, Error, Next, Request, Response, State, StatusCode,
};

use crate::support::{bench_send, request, runtime, start};

#[derive(Clone)]
struct Settings {
    greeting: String,
}

#[derive(Clone)]
struct First(i64);

#[derive(Clone)]
struct Second(i64);

#[derive(Clone)]
struct Third(i64);

impl Dependency for First {
    type Needs = ();
    type Rejection = Infallible;

    async fn resolve((): ()) -> Result<Self, Infallible> {
        Ok(Self(1))
    }
}

impl Dependency for Second {
    type Needs = Depends<First>;
    type Rejection = Infallible;

    async fn resolve(Depends(first): Depends<First>) -> Result<Self, Infallible> {
        Ok(Self(first.0 + 1))
    }
}

impl Dependency for Third {
    type Needs = Depends<Second>;
    type Rejection = Infallible;

    async fn resolve(Depends(second): Depends<Second>) -> Result<Self, Infallible> {
        Ok(Self(second.0 + 1))
    }
}

async fn plaintext() -> &'static str {
    "Hello, World!"
}

async fn depends(Depends(third): Depends<Third>) -> String {
    third.0.to_string()
}

async fn state(State(settings): State<Settings>) -> String {
    settings.greeting.clone()
}

async fn background(tasks: BackgroundTasks) -> &'static str {
    tasks.add(async {});
    "queued"
}

async fn fails() -> Result<String, Error> {
    let number: i64 = "not a number".parse()?;
    Ok(number.to_string())
}

async fn pass_through(request: Request, next: Next) -> Response {
    next.run(request).await
}

fn with_middleware(count: usize) -> App {
    (0..count).fold(App::new(), |app, _| app.middleware(pass_through))
}

fn features(criterion: &mut Criterion) {
    let runtime = runtime();
    let cases = [
        ("baseline", App::new().get("/", plaintext), StatusCode::OK),
        ("depends/3", App::new().get("/", depends), StatusCode::OK),
        (
            "state",
            App::new()
                .state(Settings {
                    greeting: "Hello, World!".to_owned(),
                })
                .get("/", state),
            StatusCode::OK,
        ),
        (
            "background_task",
            App::new().get("/", background),
            StatusCode::OK,
        ),
        (
            "middleware/1",
            with_middleware(1).get("/", plaintext),
            StatusCode::OK,
        ),
        (
            "middleware/5",
            with_middleware(5).get("/", plaintext),
            StatusCode::OK,
        ),
        (
            "exception_handler",
            App::new()
                .exception_handler(|error: &ParseIntError| {
                    (StatusCode::BAD_REQUEST, error.to_string())
                })
                .get("/", fails),
            StatusCode::BAD_REQUEST,
        ),
    ];

    let mut group = criterion.benchmark_group("features");
    for (name, app, expected) in cases {
        let server = start(&runtime, app.expect("a route that registers"));
        bench_send(&mut group, &runtime, name, &server, expected, || {
            request(thermite::http::Request::get("/"), "")
        });
    }
    group.finish();
}

criterion_group!(benches, features);
