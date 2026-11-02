use serde::Deserialize;
use serde_json::Value;
use thermite::{
    App, Method, Model, Path, RouteError, Router, StatusCode, TestClient, delete, get, head,
    options, patch, post, put, routes, trace,
};

use crate::support::TestResult;

#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: i64,
}

#[get("/items/{item_id}")]
async fn read_item(Path(path): Path<ItemPath>) -> String {
    format!("item {}", path.item_id)
}

#[get("/")]
async fn answer_get() -> &'static str {
    "GET"
}

#[post("/")]
async fn answer_post() -> &'static str {
    "POST"
}

#[put("/")]
async fn answer_put() -> &'static str {
    "PUT"
}

#[patch("/")]
async fn answer_patch() -> &'static str {
    "PATCH"
}

#[delete("/")]
async fn answer_delete() -> &'static str {
    "DELETE"
}

#[head("/")]
async fn answer_head() -> &'static str {
    "HEAD"
}

#[options("/")]
async fn answer_options() -> &'static str {
    "OPTIONS"
}

#[trace("/")]
async fn answer_trace() -> &'static str {
    "TRACE"
}

// compiled out together with its route, which no `routes!` could name
#[get("/hidden")]
#[cfg(any())]
async fn hidden() {}

#[get("/old")]
#[deprecated = "use /items/{item_id}"]
async fn old_items() -> &'static str {
    "old"
}

mod admin {
    use thermite::post;

    #[post("/reset")]
    pub async fn reset() -> &'static str {
        "reset"
    }
}

#[tokio::test]
async fn each_attribute_routes_its_method_to_its_path() -> TestResult {
    let app = App::new().routes(routes![
        answer_get,
        answer_post,
        answer_put,
        answer_patch,
        answer_delete,
        answer_head,
        answer_options,
        answer_trace,
    ])?;
    let client = TestClient::start(app).await?;

    let mut answered = Vec::new();
    for method in [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
        Method::TRACE,
    ] {
        answered.push(client.request(method, "/").send().await?.text()?.to_owned());
    }
    let head = client.head("/").send().await?;
    client.stop().await;

    assert_eq!(
        answered,
        ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE"]
    );
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers()["content-length"], "4");
    Ok(())
}

#[tokio::test]
async fn reads_the_path_parameters_of_its_path() -> TestResult {
    let client = TestClient::start(App::new().routes(routes![read_item])?).await?;

    let response = client.get("/items/7").send().await?;
    let invalid = client.get("/items/seven").send().await?;
    client.stop().await;

    assert_eq!(response.text()?, "item 7");
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    Ok(())
}

#[tokio::test]
async fn the_function_can_still_be_called_directly() {
    let answer = read_item(Path(ItemPath { item_id: 3 })).await;

    assert_eq!(answer, "item 3");
}

#[tokio::test]
async fn a_router_nests_the_routes_under_its_prefix_and_tags() -> TestResult {
    let items = Router::new()
        .prefix("/v1")
        .tag("items")
        .routes(routes![read_item, admin::reset]);
    let client = TestClient::start(App::new().include_router(items)?).await?;

    let item = client.get("/v1/items/7").send().await?;
    let reset = client.post("/v1/reset").send().await?;
    let document: Value = client.get("/openapi.json").send().await?.json()?;
    client.stop().await;

    assert_eq!(item.text()?, "item 7");
    assert_eq!(reset.text()?, "reset");
    let operation = &document["paths"]["/v1/items/{item_id}"]["get"];
    assert_eq!(operation["summary"], "Read Item");
    assert_eq!(operation["operationId"], "read_item_v1_items__item_id__get");
    assert_eq!(operation["tags"], serde_json::json!(["items"]));
    Ok(())
}

// a deprecated handler is still served, and listing it is its only use
#[tokio::test]
async fn serves_a_deprecated_handler_without_a_warning_from_its_route() -> TestResult {
    let client = TestClient::start(App::new().routes(routes![old_items])?).await?;

    let response = client.get("/old").send().await?;
    client.stop().await;

    assert_eq!(response.text()?, "old");
    Ok(())
}

#[tokio::test]
async fn sits_beside_routes_added_by_method() -> TestResult {
    let app = App::new()
        .get("/", || async { "home" })?
        .routes(routes![read_item])?
        .routes(routes![])?;
    let client = TestClient::start(app).await?;

    let home = client.get("/").send().await?;
    let item = client.get("/items/1").send().await?;
    client.stop().await;

    assert_eq!([home.text()?, item.text()?], ["home", "item 1"]);
    Ok(())
}

mod relative {
    use thermite::get;

    #[get("items")]
    pub async fn items() {}
}

#[test]
fn reports_a_path_the_core_rejects_when_the_routes_are_added() {
    let added = App::new().routes(routes![relative::items]);
    let conflict = App::new().routes(routes![answer_get, answer_get]);

    assert!(matches!(
        added,
        Err(RouteError::MissingLeadingSlash { path }) if path == "items"
    ));
    assert!(matches!(conflict, Err(RouteError::Conflict { .. })));
}
