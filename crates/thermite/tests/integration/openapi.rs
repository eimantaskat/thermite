use std::future::{Future, ready};

use serde::Deserialize;
use serde_json::json;
use thermite::http::header::CONTENT_TYPE;
use thermite::{
    App, Dependency, Depends, Field, Header, Json, Method, Model, ModelMismatch, OpenApiError,
    Path, Query, Server, ServerError, StatusCode, ToSchema, Value,
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
struct Paging {
    limit: i64,
    search: Option<String>,
}

impl Model for Paging {
    fn fields() -> Vec<Field> {
        vec![
            Field::new("limit", i64::schema()).with_default(10),
            Field::new("search", String::schema()).optional(),
        ]
    }
}

#[derive(Deserialize)]
struct Client {
    x_token: String,
}

impl Model for Client {
    fn fields() -> Vec<Field> {
        vec![Field::new("x_token", String::schema())]
    }
}

#[derive(Clone)]
struct Authorized(String);

impl Dependency for Authorized {
    type Needs = Header<Client>;
    type Rejection = ModelMismatch;

    fn resolve(
        Header(client): Header<Client>,
    ) -> impl Future<Output = Result<Self, ModelMismatch>> + Send {
        ready(Ok(Self(client.x_token)))
    }
}

#[derive(Deserialize)]
struct Tag {
    label: String,
}

impl Model for Tag {
    fn fields() -> Vec<Field> {
        vec![Field::new("label", String::schema())]
    }
}

#[derive(Deserialize)]
struct Item {
    name: String,
    tags: Vec<Tag>,
}

impl Model for Item {
    fn fields() -> Vec<Field> {
        vec![
            Field::new("name", String::schema()),
            Field::new("tags", Vec::<Tag>::schema()).with_default(json!([])),
        ]
    }
}

async fn read_item(
    Depends(Authorized(token)): Depends<Authorized>,
    Query(paging): Query<Paging>,
    Path(path): Path<ItemPath>,
) -> String {
    format!(
        "{token} {} {} {:?}",
        path.item_id, paging.limit, paging.search
    )
}

async fn create_item(Json(item): Json<Item>) -> String {
    let labels: Vec<String> = item.tags.into_iter().map(|tag| tag.label).collect();
    format!("{} {labels:?}", item.name)
}

async fn fetch(server: &RunningServer, path: &str) -> Result<reqwest::Response, reqwest::Error> {
    reqwest::get(server.url(path)).await
}

async fn document(server: &RunningServer) -> Result<Value, Box<dyn std::error::Error>> {
    let response = fetch(server, "/openapi.json").await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
    Ok(serde_json::from_str(&response.text().await?)?)
}

fn validation_error_components() -> Value {
    json!({
        "HTTPValidationError": {
            "properties": {
                "detail": {
                    "items": {"$ref": "#/components/schemas/ValidationError"},
                    "title": "Detail",
                    "type": "array"
                }
            },
            "title": "HTTPValidationError",
            "type": "object"
        },
        "ValidationError": {
            "properties": {
                "loc": {
                    "items": {"anyOf": [{"type": "string"}, {"type": "integer"}]},
                    "title": "Location",
                    "type": "array"
                },
                "msg": {"title": "Message", "type": "string"},
                "type": {"title": "Error Type", "type": "string"},
                "input": {"title": "Input"},
                "ctx": {"title": "Context", "type": "object"}
            },
            "required": ["loc", "msg", "type"],
            "title": "ValidationError",
            "type": "object"
        }
    })
}

fn validation_error_response() -> Value {
    json!({
        "description": "Validation Error",
        "content": {
            "application/json": {"schema": {"$ref": "#/components/schemas/HTTPValidationError"}}
        }
    })
}

fn successful_response() -> Value {
    json!({
        "description": "Successful Response",
        "content": {"application/json": {"schema": {}}}
    })
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // the whole document is the expected value
async fn documents_the_routes_in_registration_order_as_fastapi_does() -> TestResult {
    let app = App::new()
        .get("/items/{item_id}", read_item)?
        .post("/items", create_item)?
        .get("/", || async { "home" })?
        .delete("/items/{item_id}", |_: Path<ItemPath>| async {})?;
    let server = RunningServer::start(app).await?;

    let document = document(&server).await?;
    server.stop().await?;

    let mut expected_components = validation_error_components();
    expected_components["Item"] = json!({
        "properties": {
            "name": {"title": "Name", "type": "string"},
            "tags": {
                "default": [],
                "items": {"$ref": "#/components/schemas/Tag"},
                "title": "Tags",
                "type": "array"
            }
        },
        "required": ["name"],
        "title": "Item",
        "type": "object"
    });
    expected_components["Tag"] = json!({
        "properties": {"label": {"title": "Label", "type": "string"}},
        "required": ["label"],
        "title": "Tag",
        "type": "object"
    });
    assert_eq!(
        document,
        json!({
            "openapi": "3.1.0",
            "info": {"title": "Thermite", "version": "0.1.0"},
            "paths": {
                "/items/{item_id}": {
                    "get": {
                        "summary": "Read Item",
                        "operationId": "read_item_items__item_id__get",
                        "parameters": [
                            {
                                "name": "item_id",
                                "in": "path",
                                "required": true,
                                "schema": {"title": "Item Id", "type": "integer"}
                            },
                            {
                                "name": "limit",
                                "in": "query",
                                "required": false,
                                "schema": {"default": 10, "title": "Limit", "type": "integer"}
                            },
                            {
                                "name": "search",
                                "in": "query",
                                "required": false,
                                "schema": {
                                    "anyOf": [{"type": "string"}, {"type": "null"}],
                                    "title": "Search"
                                }
                            },
                            {
                                "name": "x-token",
                                "in": "header",
                                "required": true,
                                "schema": {"title": "X-Token", "type": "string"}
                            }
                        ],
                        "responses": {
                            "200": successful_response(),
                            "422": validation_error_response()
                        }
                    },
                    "delete": {
                        "operationId": "items__item_id__delete",
                        "parameters": [
                            {
                                "name": "item_id",
                                "in": "path",
                                "required": true,
                                "schema": {"title": "Item Id", "type": "integer"}
                            }
                        ],
                        "responses": {
                            "200": successful_response(),
                            "422": validation_error_response()
                        }
                    }
                },
                "/items": {
                    "post": {
                        "summary": "Create Item",
                        "operationId": "create_item_items_post",
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/Item"}
                                }
                            },
                            "required": true
                        },
                        "responses": {
                            "200": successful_response(),
                            "422": validation_error_response()
                        }
                    }
                },
                "/": {
                    "get": {
                        "operationId": "get",
                        "responses": {"200": successful_response()}
                    }
                }
            },
            "components": {"schemas": expected_components}
        })
    );
    Ok(())
}

#[tokio::test]
async fn an_app_without_inputs_documents_no_validation_errors() -> TestResult {
    let server = RunningServer::start(App::new().get("/", || async {})?).await?;

    let document = document(&server).await?;
    server.stop().await?;

    assert_eq!(document.get("components"), None);
    Ok(())
}

#[tokio::test]
async fn serves_swagger_ui_pointing_at_the_document() -> TestResult {
    let app = App::new()
        .title("Todos")
        .version("2.1.0")
        .openapi_path("/api/openapi.json")
        .docs_path("/swagger");
    let server = RunningServer::start(app).await?;

    let page = fetch(&server, "/swagger").await?;
    let content_type = page.headers()[CONTENT_TYPE].clone();
    let html = page.text().await?;
    let document: Value =
        serde_json::from_str(&fetch(&server, "/api/openapi.json").await?.text().await?)?;
    let default_paths = [
        fetch(&server, "/docs").await?.status(),
        fetch(&server, "/openapi.json").await?.status(),
    ];
    server.stop().await?;

    assert_eq!(content_type, "text/html; charset=utf-8");
    assert!(html.contains("<title>Todos - Swagger UI</title>"));
    assert!(html.contains(r#"url: "/api/openapi.json","#));
    assert_eq!(
        document["info"],
        json!({"title": "Todos", "version": "2.1.0"})
    );
    assert_eq!(
        default_paths,
        [StatusCode::NOT_FOUND, StatusCode::NOT_FOUND]
    );
    Ok(())
}

#[tokio::test]
async fn the_docs_and_the_document_can_be_turned_off() -> TestResult {
    let without_docs = RunningServer::start(App::new().without_docs()).await?;
    let without_openapi = RunningServer::start(App::new().without_openapi()).await?;

    let statuses = [
        fetch(&without_docs, "/docs").await?.status(),
        fetch(&without_docs, "/openapi.json").await?.status(),
        fetch(&without_openapi, "/docs").await?.status(),
        fetch(&without_openapi, "/openapi.json").await?.status(),
    ];
    without_docs.stop().await?;
    without_openapi.stop().await?;

    assert_eq!(
        statuses,
        [
            StatusCode::NOT_FOUND,
            StatusCode::OK,
            StatusCode::NOT_FOUND,
            StatusCode::NOT_FOUND
        ]
    );
    Ok(())
}

#[tokio::test]
async fn the_docs_answer_get_and_head_and_leave_other_methods_to_the_routes() -> TestResult {
    let app = App::new().post("/docs", || async { "posted" })?;
    let server = RunningServer::start(app).await?;
    let client = reqwest::Client::new();

    let head = client.head(server.url("/openapi.json")).send().await?;
    let post = client.post(server.url("/docs")).send().await?;
    let head_status = head.status();
    let posted = post.text().await?;
    server.stop().await?;

    assert_eq!(head_status, StatusCode::OK);
    assert_eq!(posted, "posted");
    Ok(())
}

#[tokio::test]
async fn starts_nothing_when_two_different_models_share_a_name() -> TestResult {
    let app = App::new()
        .post("/items", create_item)?
        .post("/other", |_: Json<OtherItem>| async {})?;

    let outcome = Server::bind("127.0.0.1:0")
        .await?
        .serve_with_shutdown(app, || ready(()))
        .await;

    let Err(ServerError::Startup(error)) = outcome else {
        return Err(format!("unexpected outcome: {outcome:?}").into());
    };
    assert_eq!(
        error.downcast_ref::<OpenApiError>(),
        Some(&OpenApiError::NameConflict {
            name: "Item".to_owned(),
            method: Method::POST,
            path: "/other".to_owned()
        })
    );
    assert_eq!(
        error.to_string(),
        "the route `POST /other` uses a schema named `Item`, but a different schema has that name"
    );
    Ok(())
}

#[tokio::test]
async fn starts_nothing_when_a_get_route_sits_on_a_docs_path() -> TestResult {
    let app = App::new().get("/docs", || async { "mine" })?;

    let outcome = Server::bind("127.0.0.1:0")
        .await?
        .serve_with_shutdown(app, || ready(()))
        .await;

    let Err(ServerError::Startup(error)) = outcome else {
        return Err(format!("unexpected outcome: {outcome:?}").into());
    };
    assert_eq!(
        error.to_string(),
        "the route `GET /docs` can never run, because the docs answer there"
    );
    Ok(())
}

// another model of the same name, as two modules can each have their own `Item`
#[derive(Deserialize)]
struct OtherItem;

impl Model for OtherItem {
    fn fields() -> Vec<Field> {
        Vec::new()
    }

    fn name() -> String {
        "Item".to_owned()
    }
}
