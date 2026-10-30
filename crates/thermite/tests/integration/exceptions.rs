use std::num::ParseIntError;

use serde::Deserialize;
use serde_json::{Value, json};
use thermite::http::HeaderValue;
use thermite::http::header::{ALLOW, CONTENT_TYPE};
use thermite::{
    App, Error, Field, HttpError, IntoResponse, Json, Model, Next, Query, Request,
    RequestValidationError, Response, StatusCode, ToSchema,
};

use crate::support::{RunningServer, TestResult};

#[derive(Deserialize)]
struct Lookup {
    id: String,
}

impl Model for Lookup {
    fn fields() -> Vec<Field> {
        vec![Field::new("id", String::schema())]
    }
}

async fn find_item(Query(lookup): Query<Lookup>) -> Result<String, Error> {
    let id: i64 = lookup.id.parse()?;
    if id == 0 {
        return Err(HttpError::new(StatusCode::NOT_FOUND, "Item not found")
            .with_header("x-error".parse()?, HeaderValue::from_static("missing"))
            .into());
    }
    Ok(format!("item {id}"))
}

async fn json_of(response: reqwest::Response) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&response.text().await?)?)
}

#[tokio::test]
async fn an_http_error_answers_its_status_detail_and_headers() -> TestResult {
    let server = RunningServer::start(App::new().get("/items", find_item)?).await?;

    let response = reqwest::get(server.url("/items?id=0")).await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers()["x-error"], "missing");
    assert_eq!(
        json_of(response).await?,
        json!({"detail": "Item not found"})
    );
    server.stop().await
}

#[tokio::test]
async fn a_handler_can_return_an_http_error_directly() -> TestResult {
    let app = App::new().get("/admin", || async {
        Err::<(), _>(HttpError::new(StatusCode::FORBIDDEN, "Not allowed"))
    })?;
    let server = RunningServer::start(app).await?;

    let response = reqwest::get(server.url("/admin")).await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(response).await?, json!({"detail": "Not allowed"}));
    server.stop().await
}

#[tokio::test]
async fn an_unhandled_error_answers_500_in_plain_text() -> TestResult {
    let server = RunningServer::start(App::new().get("/items", find_item)?).await?;

    let response = reqwest::get(server.url("/items?id=abc")).await?;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.headers()[CONTENT_TYPE],
        "text/plain; charset=utf-8"
    );
    assert_eq!(response.text().await?, "Internal Server Error");
    server.stop().await
}

#[tokio::test]
async fn a_handler_for_an_error_type_replaces_its_response() -> TestResult {
    let app = App::new()
        .exception_handler(|error: &ParseIntError| {
            (StatusCode::BAD_REQUEST, format!("not a number: {error}"))
        })
        .get("/items", find_item)?;
    let server = RunningServer::start(app).await?;

    let response = reqwest::get(server.url("/items?id=abc")).await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.text().await?,
        "not a number: invalid digit found in string"
    );
    server.stop().await
}

#[tokio::test]
async fn the_validation_error_handler_reshapes_every_422() -> TestResult {
    let app = App::new()
        .exception_handler(|error: &RequestValidationError| {
            let fields: Vec<&Value> = error.errors().iter().map(|error| &error["loc"]).collect();
            (StatusCode::BAD_REQUEST, Json(json!({ "invalid": fields })))
        })
        .get("/items", find_item)?;
    let server = RunningServer::start(app).await?;

    let response = reqwest::get(server.url("/items")).await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_of(response).await?,
        json!({"invalid": [["query", "id"]]})
    );
    server.stop().await
}

#[tokio::test]
async fn the_last_http_error_handler_sees_routing_errors_and_middleware_sees_its_answer()
-> TestResult {
    let app = App::new()
        .middleware(|request: Request, next: Next| async move {
            let mut response: Response = next.run(request).await;
            let status = HeaderValue::from(response.status().as_u16());
            response.headers_mut().insert("x-seen-status", status);
            response
        })
        .exception_handler(|_: &HttpError| StatusCode::IM_A_TEAPOT)
        .exception_handler(|error: &HttpError| {
            let detail = error.detail().as_str().unwrap_or_default().to_owned();
            let mut response = (StatusCode::GONE, detail).into_response();
            if let Some(allow) = error.headers().get(ALLOW) {
                response.headers_mut().insert(ALLOW, allow.clone());
            }
            response
        })
        .get("/items", || async { "items" })?;
    let server = RunningServer::start(app).await?;
    let client = reqwest::Client::new();

    let missing = client.get(server.url("/users")).send().await?;
    let wrong_method = client.delete(server.url("/items")).send().await?;

    assert_eq!(missing.status(), StatusCode::GONE);
    assert_eq!(missing.headers()["x-seen-status"], "410");
    assert_eq!(wrong_method.status(), StatusCode::GONE);
    assert_eq!(wrong_method.headers()[ALLOW], "GET, HEAD");
    assert_eq!(wrong_method.text().await?, "Method Not Allowed");
    server.stop().await
}

#[test]
fn an_error_shows_the_error_it_wraps() {
    let error = Error::from(HttpError::new(StatusCode::NOT_FOUND, "Item not found"));

    assert_eq!(error.to_string(), "404 Not Found: Item not found");
    assert!(format!("{error:?}").starts_with("HttpError { status: 404,"));
}
