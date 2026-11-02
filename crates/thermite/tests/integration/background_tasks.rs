// dependencies are written here the way users write them, as async fns that may not await
#![allow(clippy::unused_async_trait_impl)]

use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Deserialize;
use thermite::{
    App, Arguments, BackgroundTasks, Dependency, Depends, Error, FromRequest, HttpError, Model,
    Path, Request, State, StatusCode, TestClient,
};

use crate::support::TestResult;

// the test keeps a clone, to read what the app recorded
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn record(&self, event: impl Into<String>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.into());
    }

    fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone)]
struct Audit;

impl Dependency for Audit {
    type Needs = (State<Log>, BackgroundTasks);
    type Rejection = Infallible;

    async fn resolve((State(log), tasks): Self::Needs) -> Result<Self, Infallible> {
        log.record("audit resolved");
        tasks.add(async move { log.record("audit task") });
        Ok(Self)
    }
}

#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: i64,
}

async fn read_item(
    _: Depends<Audit>,
    Path(path): Path<ItemPath>,
    State(log): State<Log>,
    tasks: BackgroundTasks,
) -> String {
    let item_id = path.item_id;
    tasks.add(async move { log.record(format!("item {item_id} task")) });
    format!("item {item_id}")
}

async fn missing(State(log): State<Log>, tasks: BackgroundTasks) -> (StatusCode, &'static str) {
    tasks.add(async move { log.record("missing task") });
    (StatusCode::NOT_FOUND, "missing")
}

async fn refused(State(log): State<Log>, tasks: BackgroundTasks) -> Result<String, HttpError> {
    tasks.add(async move { log.record("refused task") });
    Err(HttpError::new(StatusCode::FORBIDDEN, "refused"))
}

async fn failed(State(log): State<Log>, tasks: BackgroundTasks) -> Result<String, Error> {
    tasks.add(async move { log.record("failed task") });
    Err(std::fmt::Error.into())
}

// refused only once the dependencies before it have run, unlike what the core validates
struct Unauthorized;

impl FromRequest for Unauthorized {
    type Rejection = HttpError;

    fn from_request(
        _: &Request,
        _: &Arguments,
    ) -> impl Future<Output = Result<Self, HttpError>> + Send {
        ready(Err(HttpError::new(StatusCode::UNAUTHORIZED, "who are you")))
    }
}

async fn rejected(_: Depends<Audit>, _: Unauthorized) {}

async fn items(log: &Log) -> Result<TestClient, Box<dyn std::error::Error>> {
    let app = App::new()
        .state(log.clone())
        .get("/items/{item_id}", read_item)?
        .get("/missing", missing)?
        .get("/refused", refused)?
        .get("/failed", failed)?
        .get("/rejected", rejected)?;
    Ok(TestClient::start(app).await?)
}

#[tokio::test]
async fn runs_the_tasks_of_the_handler_and_its_dependencies_after_the_response() -> TestResult {
    let log = Log::default();
    let client = items(&log).await?;

    let response = client.get("/items/7").send().await?;
    client.stop().await;

    assert_eq!(response.text()?, "item 7");
    assert_eq!(
        log.recorded(),
        ["audit resolved", "audit task", "item 7 task"]
    );
    Ok(())
}

#[tokio::test]
async fn runs_the_tasks_of_a_response_the_handler_chose_an_error_status_for() -> TestResult {
    let log = Log::default();
    let client = items(&log).await?;

    let response = client.get("/missing").send().await?;
    client.stop().await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(log.recorded(), ["missing task"]);
    Ok(())
}

#[tokio::test]
async fn runs_no_tasks_when_the_handler_answers_with_an_error() -> TestResult {
    let log = Log::default();
    let client = items(&log).await?;

    let refused = client.get("/refused").send().await?;
    let failed = client.get("/failed").send().await?;
    client.stop().await;

    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(log.recorded(), Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn runs_no_tasks_when_an_extractor_rejects_the_request() -> TestResult {
    let log = Log::default();
    let client = items(&log).await?;

    let response = client.get("/rejected").send().await?;
    client.stop().await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(log.recorded(), ["audit resolved"]);
    Ok(())
}
