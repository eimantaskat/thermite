// resources are written here the way users write them, as async fns that may not await
#![allow(clippy::unused_async_trait_impl)]

use std::any::type_name;
use std::convert::Infallible;
use std::error::Error;
use std::future::ready;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use thermite::{
    App, Arguments, Dependency, Depends, FromRequest, IntoResponse, Request, RequiredState,
    Resource, Response, Server, ServerError, StartupError, State, StatusCode,
};

use crate::support::{RunningServer, TestResult};

struct Settings {
    title: &'static str,
}

#[derive(Clone)]
struct Title(String);

impl Dependency for Title {
    type Needs = State<Settings>;
    type Rejection = Infallible;

    async fn resolve(State(settings): State<Settings>) -> Result<Self, Infallible> {
        Ok(Self(settings.title.to_uppercase()))
    }
}

#[tokio::test]
async fn hands_shared_state_to_handlers_and_dependencies() -> TestResult {
    let app = App::new().state(Settings { title: "thermite" }).get(
        "/",
        |State(settings): State<Settings>, Depends(title): Depends<Title>| async move {
            format!("{} {}", settings.title, title.0)
        },
    )?;
    let server = RunningServer::start(app).await?;

    let response = reqwest::get(server.url("/")).await?;

    assert_eq!(response.text().await?, "thermite THERMITE");
    server.stop().await
}

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<&'static str>>>);

impl Log {
    fn record(&self, event: &'static str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn recorded(&self) -> Vec<&'static str> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct Database {
    log: Arc<Log>,
}

impl Resource for Database {
    type Needs = State<Log>;
    type Error = Infallible;

    async fn start(State(log): State<Log>) -> Result<Self, Infallible> {
        log.record("start database");
        Ok(Self { log })
    }

    async fn stop(&self) {
        self.log.record("stop database");
    }
}

struct Cache {
    log: Arc<Log>,
}

impl Resource for Cache {
    type Needs = (State<Log>, State<Database>);
    type Error = Infallible;

    async fn start((State(log), _): Self::Needs) -> Result<Self, Infallible> {
        log.record("start cache");
        Ok(Self { log })
    }

    async fn stop(&self) {
        self.log.record("stop cache");
    }
}

#[tokio::test]
async fn starts_resources_in_order_before_the_first_request_and_stops_them_in_reverse() -> TestResult
{
    let log = Log::default();
    let app = App::new()
        .state(log.clone())
        .get("/", |State(cache): State<Cache>| async move {
            cache.log.record("request");
        })?
        .resource::<Database>()
        .resource::<Cache>();
    let server = RunningServer::start(app).await?;

    let response = reqwest::get(server.url("/")).await?;
    server.stop().await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        log.recorded(),
        [
            "start database",
            "start cache",
            "request",
            "stop cache",
            "stop database"
        ]
    );
    Ok(())
}

async fn startup_error(app: App) -> Result<Box<dyn Error + Send + Sync>, Box<dyn Error>> {
    // shuts down at once if startup succeeds, so the test fails rather than serving forever
    let outcome = Server::bind("127.0.0.1:0")
        .await?
        .serve_with_shutdown(app, || ready(()))
        .await;
    match outcome {
        Err(ServerError::Startup(error)) => Ok(error),
        other => Err(format!("unexpected outcome: {other:?}").into()),
    }
}

struct Unreachable;

impl Resource for Unreachable {
    type Needs = State<Log>;
    type Error = io::Error;

    async fn start(State(log): State<Log>) -> Result<Self, io::Error> {
        log.record("start unreachable");
        Err(io::Error::new(io::ErrorKind::ConnectionRefused, "refused"))
    }
}

#[tokio::test]
async fn stops_the_resources_already_started_when_one_fails_to_start() -> TestResult {
    let log = Log::default();
    let app = App::new()
        .state(log.clone())
        .resource::<Database>()
        .resource::<Unreachable>()
        .resource::<Cache>();

    let error = startup_error(app).await?;

    assert!(matches!(
        error.downcast_ref(),
        Some(StartupError::Resource { resource, .. }) if *resource == type_name::<Unreachable>()
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "the resource {} failed to start",
            type_name::<Unreachable>()
        )
    );
    assert_eq!(
        error.source().map(ToString::to_string).as_deref(),
        Some("refused")
    );
    assert_eq!(
        log.recorded(),
        ["start database", "start unreachable", "stop database"]
    );
    Ok(())
}

#[tokio::test]
async fn starts_nothing_when_a_route_needs_state_nobody_adds() -> TestResult {
    let log = Log::default();
    let app = App::new()
        .state(log.clone())
        .resource::<Database>()
        .get("/", |_: Depends<Title>| async {})?;

    let error = startup_error(app).await?;

    assert!(matches!(
        error.downcast_ref(),
        Some(StartupError::MissingState { state, needed_by })
            if *state == type_name::<Settings>() && needed_by == "GET /"
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "GET / needs the state {}, but no .state() or .resource() adds it",
            type_name::<Settings>()
        )
    );
    assert_eq!(log.recorded(), Vec::<&str>::new());
    Ok(())
}

#[tokio::test]
async fn a_resource_cannot_need_one_added_after_it() -> TestResult {
    let log = Log::default();
    let app = App::new()
        .state(log.clone())
        .resource::<Cache>()
        .resource::<Database>();

    let error = startup_error(app).await?;

    assert_eq!(
        error.to_string(),
        format!(
            "the resource {} needs {}, which is added after it",
            type_name::<Cache>(),
            type_name::<Database>()
        )
    );
    assert_eq!(log.recorded(), Vec::<&str>::new());
    Ok(())
}

struct Mailer;

impl Resource for Mailer {
    type Needs = State<Settings>;
    type Error = Infallible;

    async fn start(_: State<Settings>) -> Result<Self, Infallible> {
        Ok(Self)
    }
}

#[tokio::test]
async fn starts_nothing_when_a_resource_needs_state_nobody_adds() -> TestResult {
    let log = Log::default();
    let app = App::new()
        .state(log.clone())
        .resource::<Database>()
        .resource::<Mailer>();

    let error = startup_error(app).await?;

    assert_eq!(
        error.to_string(),
        format!(
            "the resource {} needs the state {}, but no .state() or .resource() adds it",
            type_name::<Mailer>(),
            type_name::<Settings>()
        )
    );
    assert_eq!(log.recorded(), Vec::<&str>::new());
    Ok(())
}

// an extractor of the user's own that reads state through State<T>
struct Shouted(String);

impl FromRequest for Shouted {
    type Rejection = Response;

    fn require_state(required: &mut RequiredState) {
        State::<Settings>::require_state(required);
    }

    async fn from_request(request: &Request, arguments: &Arguments) -> Result<Self, Response> {
        let State(settings) = State::<Settings>::from_request(request, arguments)
            .await
            .map_err(IntoResponse::into_response)?;
        Ok(Self(settings.title.to_uppercase()))
    }
}

#[tokio::test]
async fn checks_the_state_an_extractor_of_the_users_own_requires() -> TestResult {
    let app = App::new().get("/", |Shouted(title): Shouted| async move { title })?;

    let error = startup_error(app).await?;

    assert!(matches!(
        error.downcast_ref(),
        Some(StartupError::MissingState { state, .. }) if *state == type_name::<Settings>()
    ));
    Ok(())
}

#[tokio::test]
async fn starts_nothing_when_a_resource_is_added_twice() -> TestResult {
    let log = Log::default();
    let app = App::new()
        .state(log.clone())
        .resource::<Database>()
        .resource::<Database>();

    let also_state = App::new()
        .state(log.clone())
        .state(Database {
            log: Arc::new(log.clone()),
        })
        .resource::<Database>();

    let error = startup_error(app).await?;
    let also_state_error = startup_error(also_state).await?;

    let expected = format!(
        "the resource {} is added more than once, as a resource or as state",
        type_name::<Database>()
    );
    assert_eq!(error.to_string(), expected);
    assert_eq!(also_state_error.to_string(), expected);
    assert_eq!(log.recorded(), Vec::<&str>::new());
    Ok(())
}

struct Recursive;

impl Resource for Recursive {
    type Needs = State<Recursive>;
    type Error = Infallible;

    async fn start(_: State<Recursive>) -> Result<Self, Infallible> {
        Ok(Self)
    }
}

#[tokio::test]
async fn a_resource_cannot_need_itself() -> TestResult {
    let error = startup_error(App::new().resource::<Recursive>()).await?;

    assert_eq!(
        error.to_string(),
        format!("the resource {} needs itself", type_name::<Recursive>())
    );
    Ok(())
}
