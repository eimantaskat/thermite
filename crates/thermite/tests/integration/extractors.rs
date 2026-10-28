use serde::{Deserialize, Serialize};
use thermite::http::header::CONTENT_TYPE;
use thermite::{App, Cookie, Field, Header, Json, Model, Path, Query, StatusCode, ToSchema};

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
struct Paging {
    limit: i64,
    tags: Vec<String>,
    search: Option<String>,
}

impl Model for Paging {
    fn fields() -> Vec<Field> {
        vec![
            Field::new("limit", i64::schema()).with_default(10),
            Field::new("tags", Vec::<String>::schema()).with_default(Vec::<String>::new()),
            Field::new("search", String::schema()).optional(),
        ]
    }
}

#[derive(Deserialize)]
struct Client {
    x_token: String,
    x_tag: Vec<String>,
}

impl Model for Client {
    fn fields() -> Vec<Field> {
        vec![
            Field::new("x_token", String::schema()),
            Field::new("x_tag", Vec::<String>::schema()).with_default(Vec::<String>::new()),
        ]
    }
}

#[derive(Deserialize)]
struct Session {
    session_id: String,
}

impl Model for Session {
    fn fields() -> Vec<Field> {
        vec![Field::new("session_id", String::schema())]
    }
}

#[derive(Serialize, Deserialize)]
struct Item {
    name: String,
    price: f64,
}

impl Model for Item {
    fn fields() -> Vec<Field> {
        vec![
            Field::new("name", String::schema()),
            Field::new("price", f64::schema()),
        ]
    }
}

#[derive(Serialize)]
struct Updated {
    item_id: i64,
    limit: i64,
    tags: Vec<String>,
    search: Option<String>,
    token: String,
    client_tags: Vec<String>,
    session: String,
    item: Item,
}

async fn update_item(
    Path(path): Path<ItemPath>,
    Query(paging): Query<Paging>,
    Header(client): Header<Client>,
    Cookie(session): Cookie<Session>,
    Json(item): Json<Item>,
) -> Json<Updated> {
    Json(Updated {
        item_id: path.item_id,
        limit: paging.limit,
        tags: paging.tags,
        search: paging.search,
        token: client.x_token,
        client_tags: client.x_tag,
        session: session.session_id,
        item,
    })
}

fn update_request(
    client: &reqwest::Client,
    server: &RunningServer,
    path: &str,
) -> reqwest::RequestBuilder {
    client
        .put(server.url(path))
        .header("x-token", "secret")
        .header("cookie", "theme=dark; session_id=abc")
        .body(r#"{"name": "anvil", "price": "9.5"}"#)
}

#[tokio::test]
async fn reads_each_location_into_its_model_and_answers_with_json() -> TestResult {
    let server = RunningServer::start(App::new().put("/items/{item_id}", update_item)?).await?;
    let client = reqwest::Client::new();

    let response = update_request(&client, &server, "/items/5?limit=3&tags=a&tags=b&search=x")
        .header("x-tag", "red")
        .header("x-tag", "blue")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
    assert_eq!(
        response.text().await?,
        concat!(
            r#"{"item_id":5,"limit":3,"tags":["a","b"],"search":"x","token":"secret","#,
            r#""client_tags":["red","blue"],"session":"abc","item":{"name":"anvil","price":9.5}}"#
        )
    );
    server.stop().await
}

#[tokio::test]
async fn fills_missing_fields_with_their_defaults_and_none() -> TestResult {
    let server = RunningServer::start(App::new().put("/items/{item_id}", update_item)?).await?;

    let response = update_request(&reqwest::Client::new(), &server, "/items/5")
        .send()
        .await?;

    assert_eq!(
        response.text().await?,
        concat!(
            r#"{"item_id":5,"limit":10,"tags":[],"search":null,"token":"secret","#,
            r#""client_tags":[],"session":"abc","item":{"name":"anvil","price":9.5}}"#
        )
    );
    server.stop().await
}

#[tokio::test]
async fn reports_missing_headers_and_cookies_by_their_names_in_the_request() -> TestResult {
    let server = RunningServer::start(App::new().put("/items/{item_id}", update_item)?).await?;

    let response = reqwest::Client::new()
        .put(server.url("/items/5"))
        .body(r#"{"name": "anvil", "price": 9.5}"#)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.text().await?,
        concat!(
            r#"{"detail":["#,
            r#"{"type":"missing","loc":["header","x-token"],"msg":"Field required","input":null},"#,
            r#"{"type":"missing","loc":["cookie","session_id"],"msg":"Field required","input":null}"#,
            r"]}"
        )
    );
    server.stop().await
}

#[tokio::test]
async fn answers_422_for_an_integer_too_large_for_an_i64() -> TestResult {
    let server = RunningServer::start(App::new().put("/items/{item_id}", update_item)?).await?;

    let response = update_request(
        &reqwest::Client::new(),
        &server,
        "/items/5?limit=18446744073709551615",
    )
    .send()
    .await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.text().await?,
        concat!(
            r#"{"detail":[{"type":"int_parsing_size","loc":["query","limit"],"#,
            r#""msg":"Unable to parse input string as an integer, exceeded maximum size","#,
            r#""input":"18446744073709551615"}]}"#
        )
    );
    server.stop().await
}

#[derive(Deserialize)]
struct Mismatched {
    id: String,
}

impl Model for Mismatched {
    fn fields() -> Vec<Field> {
        vec![Field::new("id", i64::schema())]
    }
}

#[tokio::test]
async fn answers_500_when_a_model_and_its_serde_impl_disagree() -> TestResult {
    let app = App::new().get("/items", |Query(item): Query<Mismatched>| async { item.id })?;
    let server = RunningServer::start(app).await?;

    let response = reqwest::get(server.url("/items?id=5")).await?;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.text().await?, "Internal Server Error");
    server.stop().await
}

#[test]
fn rejects_a_route_with_two_bodies() {
    let error = App::new()
        .post("/items", |_: Json<Item>, _: Json<Item>| async {})
        .map(|_| ())
        .map_err(|error| error.to_string());

    assert_eq!(
        error,
        Err("`/items` declares more than one body".to_owned())
    );
}
