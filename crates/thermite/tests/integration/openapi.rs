use std::future::{Future, ready};

use serde::{Deserialize, Serialize};
use serde_json::json;
use thermite::http::header::CONTENT_TYPE;
use thermite::{
    App, Dependency, Depends, Field, Handler, Header, IntoResponse, Json, Method, Model,
    ModelMismatch, OpenApiError, Path, Query, RequestValidationError, Response,
    ResponseDescription, Server, ServerError, StatusCode, ToSchema, Value,
};

use crate::support::{RunningServer, TestResult};

#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: i64,
}

#[derive(Deserialize, Model)]
struct Paging {
    #[model(default = 10)]
    limit: i64,
    search: Option<String>,
}

#[derive(Deserialize, Model)]
struct Client {
    x_token: String,
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

#[derive(Serialize, Deserialize, Model)]
struct Tag {
    label: String,
}

#[derive(Deserialize, Model)]
struct Item {
    name: String,
    #[model(default = json!([]))]
    tags: Vec<Tag>,
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

fn content_too_large_response() -> Value {
    json!({
        "description": "Content Too Large",
        "content": {
            "application/json": {
                "schema": {
                    "properties": {"detail": {"title": "Detail", "type": "string"}},
                    "required": ["detail"],
                    "type": "object"
                }
            }
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
                            "200": {"description": "Successful Response"},
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
                            "413": content_too_large_response(),
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

struct NotOwner;

impl IntoResponse for NotOwner {
    fn describe() -> Vec<ResponseDescription> {
        vec![ResponseDescription::new(
            StatusCode::FORBIDDEN,
            "Not the owner",
        )]
    }

    fn into_response(self) -> Response {
        StatusCode::FORBIDDEN.into_response()
    }
}

#[derive(Clone)]
struct Owner;

impl Dependency for Owner {
    type Needs = Header<Client>;
    type Rejection = NotOwner;

    fn resolve(
        Header(client): Header<Client>,
    ) -> impl Future<Output = Result<Self, NotOwner>> + Send {
        ready(if client.x_token == "owner" {
            Ok(Self)
        } else {
            Err(NotOwner)
        })
    }
}

struct TagNotFound;

impl IntoResponse for TagNotFound {
    fn describe() -> Vec<ResponseDescription> {
        vec![ResponseDescription::new(
            StatusCode::NOT_FOUND,
            "Tag not found",
        )]
    }

    fn into_response(self) -> Response {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn read_tag(_: Depends<Owner>, Path(path): Path<ItemPath>) -> Result<Json<Tag>, TagNotFound> {
    if path.item_id == 1 {
        Ok(Json(Tag {
            label: "hot".to_owned(),
        }))
    } else {
        Err(TagNotFound)
    }
}

#[tokio::test]
async fn documents_what_the_types_of_a_route_answer() -> TestResult {
    let server = RunningServer::start(App::new().get("/tags/{item_id}", read_tag)?).await?;
    let client = reqwest::Client::new();
    let mut statuses = Vec::new();
    for (path, token) in [
        ("/tags/1", "owner"),
        ("/tags/2", "owner"),
        ("/tags/1", "guest"),
    ] {
        let response = client
            .get(server.url(path))
            .header("x-token", token)
            .send()
            .await?;
        statuses.push(response.status());
    }

    let document = document(&server).await?;
    server.stop().await?;

    let responses = &document["paths"]["/tags/{item_id}"]["get"]["responses"];
    let order: Vec<&String> = responses
        .as_object()
        .ok_or("no responses")?
        .keys()
        .collect();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::NOT_FOUND, StatusCode::FORBIDDEN]
    );
    assert_eq!(order, ["200", "404", "403", "422"]);
    assert_eq!(
        responses,
        &json!({
            "200": {
                "description": "Successful Response",
                "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Tag"}}}
            },
            "404": {"description": "Tag not found"},
            "403": {"description": "Not the owner"},
            "422": validation_error_response()
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

async fn read_thing(Path(path): Path<ItemPath>) -> Response {
    if path.item_id == 1 {
        Json(Tag {
            label: "hot".to_owned(),
        })
        .into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

#[tokio::test]
async fn documents_responses_added_by_hand_to_a_route_that_still_runs() -> TestResult {
    let handler = read_thing
        .response_with_body::<Tag>(StatusCode::OK, "The tag")
        .response(StatusCode::NOT_FOUND, "Tag not found");
    let server = RunningServer::start(App::new().get("/things/{item_id}", handler)?).await?;

    let found = fetch(&server, "/things/1").await?.text().await?;
    let missing = fetch(&server, "/things/2").await?.status();
    let document = document(&server).await?;
    server.stop().await?;

    let operation = &document["paths"]["/things/{item_id}"]["get"];
    assert_eq!(found, r#"{"label":"hot"}"#);
    assert_eq!(missing, StatusCode::NOT_FOUND);
    assert_eq!(operation["summary"], "Read Thing");
    assert_eq!(
        operation["responses"],
        json!({
            "200": {
                "description": "The tag",
                "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Tag"}}}
            },
            "404": {"description": "Tag not found"},
            "422": validation_error_response()
        })
    );
    Ok(())
}

#[derive(Serialize, Model)]
struct Problem {
    message: String,
}

struct BadInput(String);

impl IntoResponse for BadInput {
    fn describe() -> Vec<ResponseDescription> {
        vec![
            ResponseDescription::new(StatusCode::BAD_REQUEST, "Bad input")
                .with_schema(Problem::schema()),
        ]
    }

    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(Problem { message: self.0 })).into_response()
    }
}

#[tokio::test]
async fn the_latest_validation_error_handler_replaces_the_422_in_the_docs() -> TestResult {
    let app = App::new()
        .exception_handler(|_: &RequestValidationError| StatusCode::UNPROCESSABLE_ENTITY)
        .exception_handler(|error: &RequestValidationError| BadInput(error.to_string()))
        .post("/items", create_item)?;
    let server = RunningServer::start(app).await?;

    let status = reqwest::Client::new()
        .post(server.url("/items"))
        .body("{}")
        .send()
        .await?
        .status();
    let document = document(&server).await?;
    server.stop().await?;

    let schemas: Vec<&String> = document["components"]["schemas"]
        .as_object()
        .ok_or("no schemas")?
        .keys()
        .collect();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        document["paths"]["/items"]["post"]["responses"],
        json!({
            "200": successful_response(),
            "400": {
                "description": "Bad input",
                "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Problem"}}}
            },
            "413": content_too_large_response()
        })
    );
    assert_eq!(schemas, ["Item", "Problem", "Tag"]);
    Ok(())
}
