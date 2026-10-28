// dependencies are written here the way users write them, as async fns that may not await
#![allow(clippy::unused_async_trait_impl)]

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Deserialize;
use thermite::{
    App, Arguments, Dependency, Depends, Field, FromRequest, Header, Json, Model, Path, Query,
    Request, Response, StatusCode, ToSchema,
};

use crate::support::{RunningServer, TestResult};

#[derive(Deserialize)]
struct Paging {
    limit: i64,
}

impl Model for Paging {
    fn fields() -> Vec<Field> {
        vec![Field::new("limit", i64::schema()).with_default(10)]
    }
}

#[derive(Clone)]
struct Pagination {
    limit: i64,
}

impl Dependency for Pagination {
    type Needs = Query<Paging>;
    type Rejection = Infallible;

    async fn resolve(Query(paging): Query<Paging>) -> Result<Self, Infallible> {
        Ok(Self {
            limit: paging.limit,
        })
    }
}

#[tokio::test]
async fn hands_the_value_of_a_dependency_to_the_handler() -> TestResult {
    let app = App::new().get(
        "/items",
        |Depends(pagination): Depends<Pagination>| async move { pagination.limit.to_string() },
    )?;
    let server = RunningServer::start(app).await?;

    let given = reqwest::get(server.url("/items?limit=3")).await?;
    let defaulted = reqwest::get(server.url("/items")).await?;

    assert_eq!(given.text().await?, "3");
    assert_eq!(defaulted.text().await?, "10");
    server.stop().await
}

#[derive(Clone)]
struct Settings {
    api_key: String,
}

impl Dependency for Settings {
    type Needs = ();
    type Rejection = Infallible;

    async fn resolve((): ()) -> Result<Self, Infallible> {
        Ok(Self {
            api_key: "secret".to_owned(),
        })
    }
}

#[derive(Deserialize)]
struct ApiKey {
    x_api_key: String,
}

impl Model for ApiKey {
    fn fields() -> Vec<Field> {
        vec![Field::new("x_api_key", String::schema())]
    }
}

#[derive(Clone)]
struct CurrentUser {
    name: String,
}

impl Dependency for CurrentUser {
    type Needs = (Header<ApiKey>, Depends<Settings>);
    type Rejection = StatusCode;

    async fn resolve((Header(key), Depends(settings)): Self::Needs) -> Result<Self, StatusCode> {
        if key.x_api_key == settings.api_key {
            Ok(Self {
                name: "admin".to_owned(),
            })
        } else {
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

#[tokio::test]
async fn a_dependency_can_need_other_dependencies() -> TestResult {
    let app = App::new().get("/me", |Depends(user): Depends<CurrentUser>| async move {
        user.name
    })?;
    let server = RunningServer::start(app).await?;
    let client = reqwest::Client::new();

    let response = client
        .get(server.url("/me"))
        .header("x-api-key", "secret")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text().await?, "admin");
    server.stop().await
}

#[tokio::test]
async fn answers_with_the_rejection_of_a_dependency_without_calling_the_handler() -> TestResult {
    let handler_runs = Arc::new(AtomicUsize::new(0));
    let app = App::new().get("/me", {
        let handler_runs = Arc::clone(&handler_runs);
        move |_: Depends<CurrentUser>| {
            handler_runs.fetch_add(1, Ordering::SeqCst);
            async {}
        }
    })?;
    let server = RunningServer::start(app).await?;
    let client = reqwest::Client::new();

    let response = client
        .get(server.url("/me"))
        .header("x-api-key", "wrong")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(handler_runs.load(Ordering::SeqCst), 0);
    server.stop().await
}

static REQUEST_NUMBER_RUNS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
struct RequestNumber(usize);

impl Dependency for RequestNumber {
    type Needs = ();
    type Rejection = Infallible;

    async fn resolve((): ()) -> Result<Self, Infallible> {
        Ok(Self(REQUEST_NUMBER_RUNS.fetch_add(1, Ordering::SeqCst) + 1))
    }
}

#[derive(Clone)]
struct First(usize);

impl Dependency for First {
    type Needs = Depends<RequestNumber>;
    type Rejection = Infallible;

    async fn resolve(Depends(number): Depends<RequestNumber>) -> Result<Self, Infallible> {
        Ok(Self(number.0))
    }
}

#[derive(Clone)]
struct Second(usize);

impl Dependency for Second {
    type Needs = Depends<RequestNumber>;
    type Rejection = Infallible;

    async fn resolve(Depends(number): Depends<RequestNumber>) -> Result<Self, Infallible> {
        Ok(Self(number.0))
    }
}

#[tokio::test]
async fn runs_a_dependency_once_per_request_however_many_need_it() -> TestResult {
    let app = App::new().get(
        "/numbers",
        |Depends(first): Depends<First>,
         Depends(second): Depends<Second>,
         Depends(direct): Depends<RequestNumber>| async move {
            format!("{} {} {}", first.0, second.0, direct.0)
        },
    )?;
    let server = RunningServer::start(app).await?;

    let first_request = reqwest::get(server.url("/numbers")).await?;
    let second_request = reqwest::get(server.url("/numbers")).await?;

    assert_eq!(first_request.text().await?, "1 1 1");
    assert_eq!(second_request.text().await?, "2 2 2");
    server.stop().await
}

#[derive(Deserialize)]
struct ItemPath {
    item_id: i64,
}

impl Model for ItemPath {
    fn fields() -> Vec<Field> {
        vec![Field::new("item_id", i64::schema())]
    }
}

#[tokio::test]
async fn validates_what_dependencies_declare_before_any_of_them_runs() -> TestResult {
    let app =
        App::new().get(
            "/items/{item_id}",
            |_: Depends<CurrentUser>, Path(path): Path<ItemPath>| async move {
                path.item_id.to_string()
            },
        )?;
    let server = RunningServer::start(app).await?;

    let response = reqwest::get(server.url("/items/abc")).await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.text().await?,
        concat!(
            r#"{"detail":[{"type":"missing","loc":["header","x-api-key"],"msg":"Field required","input":null},"#,
            r#"{"type":"int_parsing","loc":["path","item_id"],"#,
            r#""msg":"Input should be a valid integer, unable to parse string as an integer","input":"abc"}]}"#
        )
    );
    server.stop().await
}

#[derive(Deserialize)]
struct Item {
    name: String,
}

impl Model for Item {
    fn fields() -> Vec<Field> {
        vec![Field::new("name", String::schema())]
    }
}

#[derive(Clone)]
struct ItemName(String);

impl Dependency for ItemName {
    type Needs = (Json<Item>, Query<Paging>);
    type Rejection = Infallible;

    async fn resolve((Json(item), _): Self::Needs) -> Result<Self, Infallible> {
        Ok(Self(item.name))
    }
}

#[tokio::test]
async fn a_dependency_and_its_handler_can_read_the_same_values() -> TestResult {
    let app =
        App::new().post(
            "/items",
            |Depends(name): Depends<ItemName>,
             Json(item): Json<Item>,
             Query(paging): Query<Paging>| async move {
                format!("{} {} {}", name.0, item.name, paging.limit)
            },
        )?;
    let server = RunningServer::start(app).await?;
    let client = reqwest::Client::new();

    let response = client
        .post(server.url("/items?limit=5"))
        .header("content-type", "application/json")
        .body(r#"{"name": "iron"}"#)
        .send()
        .await?;

    assert_eq!(response.text().await?, "iron iron 5");
    server.stop().await
}

static SLOW_RUNS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
struct Slow;

impl Dependency for Slow {
    type Needs = ();
    type Rejection = Infallible;

    async fn resolve((): ()) -> Result<Self, Infallible> {
        tokio::task::yield_now().await;
        SLOW_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(Self)
    }
}

// an extractor of the user's own that needs the same dependency twice, at the same time
struct BothAtOnce;

impl FromRequest for BothAtOnce {
    type Rejection = Response;

    async fn from_request(request: &Request, arguments: &Arguments) -> Result<Self, Response> {
        let (first, second) = tokio::join!(
            Depends::<Slow>::from_request(request, arguments),
            Depends::<Slow>::from_request(request, arguments)
        );
        first.and(second).map(|_| Self)
    }
}

#[tokio::test]
async fn runs_a_dependency_once_when_two_extractors_need_it_at_the_same_time() -> TestResult {
    let app = App::new().get("/slow", |_: BothAtOnce| async {})?;
    let server = RunningServer::start(app).await?;

    let response = reqwest::get(server.url("/slow")).await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(SLOW_RUNS.load(Ordering::SeqCst), 1);
    server.stop().await
}

// reads the header itself rather than declaring it, so a missing role is a rejection, not a 422
struct Admin;

impl FromRequest for Admin {
    type Rejection = StatusCode;

    async fn from_request(request: &Request, _: &Arguments) -> Result<Self, StatusCode> {
        if request
            .headers()
            .get("x-role")
            .is_some_and(|role| role == "admin")
        {
            Ok(Self)
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }
}

#[derive(Clone)]
struct AdminArea;

impl Dependency for AdminArea {
    type Needs = Admin;
    type Rejection = Infallible;

    async fn resolve(_: Admin) -> Result<Self, Infallible> {
        Ok(Self)
    }
}

#[tokio::test]
async fn answers_with_the_rejection_of_what_a_dependency_needs() -> TestResult {
    let app = App::new().get("/admin", |_: Depends<AdminArea>| async { "welcome" })?;
    let server = RunningServer::start(app).await?;
    let client = reqwest::Client::new();

    let denied = client.get(server.url("/admin")).send().await?;
    let allowed = client
        .get(server.url("/admin"))
        .header("x-role", "admin")
        .send()
        .await?;

    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(allowed.text().await?, "welcome");
    server.stop().await
}
