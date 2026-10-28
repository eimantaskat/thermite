use std::future::{Future, ready};

use thermite::http::HeaderValue;
use thermite::{
    App, Arguments, FromRequest, Handler, Method, Parameter, Request, Response, RouteDescription,
    Schema, StatusCode, Value,
};

use crate::support::{RunningServer, TestResult};

fn text(status: StatusCode, body: impl Into<String>) -> Response {
    let mut response = Response::new(body.into().into());
    *response.status_mut() = status;
    response
}

struct ItemId(i64);

impl FromRequest for ItemId {
    type Rejection = Response;

    fn describe(description: RouteDescription) -> RouteDescription {
        description.parameter(Parameter::path("id", Schema::integer()))
    }

    fn from_request(
        _: &Request,
        arguments: &Arguments,
    ) -> impl Future<Output = Result<Self, Response>> + Send {
        ready(
            arguments
                .path("id")
                .and_then(Value::as_i64)
                .map(Self)
                .ok_or_else(|| text(StatusCode::INTERNAL_SERVER_ERROR, "no id")),
        )
    }
}

struct Limit(i64);

impl FromRequest for Limit {
    type Rejection = Response;

    fn describe(description: RouteDescription) -> RouteDescription {
        description.parameter(Parameter::query("limit", Schema::integer()).with_default(10))
    }

    fn from_request(
        _: &Request,
        arguments: &Arguments,
    ) -> impl Future<Output = Result<Self, Response>> + Send {
        ready(
            arguments
                .query("limit")
                .and_then(Value::as_i64)
                .map(Self)
                .ok_or_else(|| text(StatusCode::INTERNAL_SERVER_ERROR, "no limit")),
        )
    }
}

struct Forbidden;

impl From<Forbidden> for Response {
    fn from(_: Forbidden) -> Self {
        text(StatusCode::FORBIDDEN, r#"{"detail":"Not authenticated"}"#)
    }
}

struct Admin;

impl FromRequest for Admin {
    type Rejection = Forbidden;

    fn from_request(
        request: &Request,
        _: &Arguments,
    ) -> impl Future<Output = Result<Self, Forbidden>> + Send {
        let is_admin = request
            .headers()
            .get("x-role")
            .is_some_and(|role| role == "admin");
        ready(if is_admin { Ok(Self) } else { Err(Forbidden) })
    }
}

async fn read_item(ItemId(id): ItemId, Limit(limit): Limit) -> Response {
    text(StatusCode::OK, format!("item {id}, limit {limit}"))
}

fn answering(name: &'static str) -> impl Handler<()> {
    move || async move {
        let mut response = text(StatusCode::OK, name);
        response
            .headers_mut()
            .insert("x-handler", HeaderValue::from_static(name));
        response
    }
}

#[tokio::test]
async fn calls_an_async_fn_with_the_values_its_extractors_declared() -> TestResult {
    let server = RunningServer::start(App::new().get("/items/{id}", read_item)?).await?;

    let with_limit = reqwest::get(server.url("/items/5?limit=3")).await?;
    let with_default = reqwest::get(server.url("/items/5")).await?;

    assert_eq!(with_limit.status(), StatusCode::OK);
    assert_eq!(with_limit.text().await?, "item 5, limit 3");
    assert_eq!(with_default.text().await?, "item 5, limit 10");
    server.stop().await
}

#[tokio::test]
async fn answers_422_before_the_handler_when_a_declared_value_is_invalid() -> TestResult {
    let server = RunningServer::start(App::new().get("/items/{id}", read_item)?).await?;

    let response = reqwest::get(server.url("/items/x?limit=y")).await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.text().await?,
        concat!(
            r#"{"detail":["#,
            r#"{"type":"int_parsing","loc":["path","id"],"msg":"Input should be a valid integer, unable to parse string as an integer","input":"x"},"#,
            r#"{"type":"int_parsing","loc":["query","limit"],"msg":"Input should be a valid integer, unable to parse string as an integer","input":"y"}"#,
            r"]}"
        )
    );
    server.stop().await
}

#[tokio::test]
async fn answers_with_the_rejection_of_a_failing_extractor() -> TestResult {
    let app = App::new().delete("/items/{id}", |_: Admin, ItemId(id): ItemId| async move {
        text(StatusCode::OK, format!("deleted {id}"))
    })?;
    let server = RunningServer::start(app).await?;
    let client = reqwest::Client::new();

    let anonymous = client.delete(server.url("/items/5")).send().await?;
    let admin = client
        .delete(server.url("/items/5"))
        .header("x-role", "admin")
        .send()
        .await?;

    assert_eq!(anonymous.status(), StatusCode::FORBIDDEN);
    assert_eq!(anonymous.text().await?, r#"{"detail":"Not authenticated"}"#);
    assert_eq!(admin.text().await?, "deleted 5");
    server.stop().await
}

#[tokio::test]
async fn answers_422_before_an_extractor_could_reject() -> TestResult {
    let app = App::new().delete("/items/{id}", |_: Admin, _: ItemId| async {
        Response::default()
    })?;
    let server = RunningServer::start(app).await?;

    let response = reqwest::Client::new()
        .delete(server.url("/items/x"))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    server.stop().await
}

#[tokio::test]
async fn routes_each_method_helper_to_its_own_method() -> TestResult {
    let app = App::new()
        .get("/resource", answering("GET"))?
        .post("/resource", answering("POST"))?
        .put("/resource", answering("PUT"))?
        .patch("/resource", answering("PATCH"))?
        .delete("/resource", answering("DELETE"))?
        .head("/resource", answering("HEAD"))?
        .options("/resource", answering("OPTIONS"))?
        .trace("/resource", answering("TRACE"))?
        .route(
            Method::from_bytes(b"PURGE")?,
            "/resource",
            answering("PURGE"),
        )?;
    let server = RunningServer::start(app).await?;
    let client = reqwest::Client::new();

    for method in [
        "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "PURGE",
    ] {
        let response = client
            .request(
                Method::from_bytes(method.as_bytes())?,
                server.url("/resource"),
            )
            .send()
            .await?;

        assert_eq!(response.headers()["x-handler"], method);
    }
    server.stop().await
}

#[test]
fn rejects_routes_whose_extractors_do_not_fit_the_path() {
    let errors = [
        App::new().get("/items", read_item),
        App::new().get("/items/{id}", |_: ItemId, _: ItemId| async {
            Response::default()
        }),
        App::new().get("/items/{id}", || async { Response::default() }),
    ]
    .map(|result| result.map(|_| ()).map_err(|error| error.to_string()));

    assert_eq!(
        errors,
        [
            Err("path `/items` has no segment `{id}` for its path parameter `id`".to_owned()),
            Err("path parameter `id` of `/items/{id}` is declared more than once".to_owned()),
            Err("path `/items/{id}` has the segment `{id}` but no path parameter `id`".to_owned()),
        ]
    );
}
