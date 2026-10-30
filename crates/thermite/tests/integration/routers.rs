use std::future::ready;

use serde::Deserialize;
use serde_json::{Value, json};
use thermite::{
    App, Field, Method, Model, Path, RouteError, Router, Server, ServerError, State, ToSchema,
};

use crate::support::{RunningServer, TestResult};

#[derive(Deserialize)]
struct ItemPath {
    item_id: i64,
}

impl Model for ItemPath {
    fn fields() -> Vec<Field> {
        vec![Field::new("item_id", i64::schema())]
    }
}

#[derive(Deserialize)]
struct UserItemPath {
    user_id: String,
    item_id: i64,
}

impl Model for UserItemPath {
    fn fields() -> Vec<Field> {
        vec![
            Field::new("user_id", String::schema()),
            Field::new("item_id", i64::schema()),
        ]
    }
}

async fn read_item(Path(path): Path<ItemPath>) -> String {
    format!("item {}", path.item_id)
}

async fn read_user_item(Path(path): Path<UserItemPath>) -> String {
    format!("{} owns {}", path.user_id, path.item_id)
}

#[tokio::test]
async fn serves_the_routes_of_a_router_under_its_prefix() -> TestResult {
    let items = Router::new()
        .prefix("/items")
        .get("", || async { "all items" })
        .get("/{item_id}", read_item);
    let app = App::new()
        .get("/", || async { "home" })?
        .include_router(items)?;
    let server = RunningServer::start(app).await?;

    let home = reqwest::get(server.url("/")).await?.text().await?;
    let all = reqwest::get(server.url("/items")).await?.text().await?;
    let one = reqwest::get(server.url("/items/5")).await?.text().await?;

    assert_eq!([home, all, one], ["home", "all items", "item 5"]);
    server.stop().await
}

#[tokio::test]
async fn a_nested_route_reads_path_parameters_from_every_prefix() -> TestResult {
    let items = Router::new()
        .prefix("/items")
        .get("/{item_id}", read_user_item);
    let users = Router::new()
        .prefix("/users/{user_id}")
        .include_router(items);
    let server = RunningServer::start(App::new().include_router(users)?).await?;

    let response = reqwest::get(server.url("/users/ada/items/7")).await?;

    assert_eq!(response.text().await?, "ada owns 7");
    server.stop().await
}

#[tokio::test]
async fn tags_operations_with_the_outer_router_tags_first() -> TestResult {
    let items = Router::new()
        .prefix("/items")
        .tag("items")
        .get("/{item_id}", read_item);
    let shop = Router::new()
        .prefix("/shop")
        .tag("shop")
        .tag("public")
        .include_router(items);
    let server = RunningServer::start(App::new().include_router(shop)?).await?;

    let response = reqwest::get(server.url("/openapi.json")).await?;
    let document: Value = serde_json::from_str(&response.text().await?)?;
    let operation = &document["paths"]["/shop/items/{item_id}"]["get"];

    assert_eq!(operation["tags"], json!(["shop", "public", "items"]));
    assert_eq!(
        operation["operationId"],
        "read_item_shop_items__item_id__get"
    );
    server.stop().await
}

#[test]
fn reports_a_bad_route_with_its_full_path_when_included() {
    let undeclared = Router::new()
        .prefix("/users/{user_id}")
        .get("/items/{item_id}", read_item);
    let bad_prefix = Router::new().prefix("/items/").get("", || async {});

    let errors = [undeclared, bad_prefix].map(|router| App::new().include_router(router).err());

    assert_eq!(
        errors,
        [
            Some(RouteError::UndeclaredPathParameter {
                path: "/users/{user_id}/items/{item_id}".to_owned(),
                name: "user_id".to_owned(),
            }),
            Some(RouteError::InvalidPrefix {
                prefix: "/items/".to_owned(),
                path: "/items/".to_owned(),
            }),
        ]
    );
}

#[tokio::test]
async fn names_the_full_path_of_a_router_route_that_needs_missing_state() -> TestResult {
    let router = Router::new()
        .prefix("/items")
        .get("/count", |_: State<u32>| async {});
    let app = App::new().include_router(router)?;

    let outcome = Server::bind("127.0.0.1:0")
        .await?
        .serve_with_shutdown(app, || ready(()))
        .await;

    let Err(ServerError::Startup(error)) = outcome else {
        return Err(format!("unexpected outcome: {outcome:?}").into());
    };
    assert_eq!(
        error.to_string(),
        "GET /items/count needs the state u32, but no .state() or .resource() adds it"
    );
    Ok(())
}

#[test]
fn debug_output_shows_prefix_tags_and_routes() {
    let router =
        Router::new()
            .prefix("/items")
            .tag("items")
            .route(Method::PUT, "/{item_id}", read_item);

    assert_eq!(
        format!("{router:?}"),
        r#"Router { prefix: "/items", tags: ["items"], routes: [PUT /{item_id}] }"#
    );
}
