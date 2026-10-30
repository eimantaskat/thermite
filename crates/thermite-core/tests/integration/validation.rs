use bytes::Bytes;
use http::{Method, StatusCode};
use serde_json::{Value, json};
use thermite_core::{
    Arguments, Field, Parameter, Request, Response, RouteDescription, RouteHandler, Schema,
};

use crate::support::{RunningServer, TestResult, one_route, text_response};

fn answering_called() -> impl RouteHandler {
    |_request: Request, _arguments: Arguments| async { text_response(StatusCode::OK, "called") }
}

fn item_schema() -> Schema {
    Schema::object([
        Field::new("name", Schema::string()),
        Field::new("price", Schema::number()),
        Field::new("tags", Schema::array(Schema::string())).optional(),
    ])
}

async fn post_json(
    server: &RunningServer,
    path: &str,
    body: &'static str,
) -> reqwest::Result<reqwest::Response> {
    reqwest::Client::new()
        .post(server.url(path))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
}

async fn json_body(response: reqwest::Response) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&response.text().await?)?)
}

#[tokio::test]
async fn passes_validated_path_query_and_body_values_to_the_handler() -> TestResult {
    let description = RouteDescription::new(Method::POST, "/items/{item_id}")
        .parameter(Parameter::path("item_id", Schema::integer()))
        .parameter(Parameter::query("q", Schema::string()).optional())
        .parameter(Parameter::query("tag", Schema::array(Schema::string())))
        .parameter(Parameter::query("limit", Schema::integer()).with_default(10))
        .parameter(Parameter::query("active", Schema::boolean()))
        .body(item_schema());
    let router = one_route(
        description,
        |_request: Request, arguments: Arguments| async move {
            let echoed = json!({
                "item_id": arguments.path("item_id"),
                "q": arguments.query("q"),
                "tag": arguments.query("tag"),
                "limit": arguments.query("limit"),
                "active": arguments.query("active"),
                "body": arguments.body(),
            });
            Response::new(Bytes::from(echoed.to_string()))
        },
    )?;
    let server = RunningServer::start(router).await?;

    let response = post_json(
        &server,
        "/items/5?tag=hot&active=yes&tag=fast&limit=4.0",
        r#"{"name": "anvil", "price": "9.5", "colour": "black"}"#,
    )
    .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await?,
        json!({
            "item_id": 5,
            "q": null,
            "tag": ["hot", "fast"],
            "limit": 4,
            "active": true,
            "body": {"name": "anvil", "price": 9.5, "tags": null},
        })
    );
    server.stop().await
}

#[tokio::test]
async fn returns_422_in_fastapi_format_when_a_path_parameter_does_not_parse() -> TestResult {
    let description = RouteDescription::new(Method::GET, "/items/{item_id}")
        .parameter(Parameter::path("item_id", Schema::integer()));
    let server = RunningServer::start(one_route(description, answering_called())?).await?;

    let response = reqwest::get(server.url("/items/abc")).await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(
        response.text().await?,
        concat!(
            r#"{"detail":[{"type":"int_parsing","loc":["path","item_id"],"#,
            r#""msg":"Input should be a valid integer, unable to parse string as an integer","input":"abc"}]}"#
        )
    );
    server.stop().await
}

#[tokio::test]
async fn lists_every_invalid_query_and_body_value_in_one_response() -> TestResult {
    let description = RouteDescription::new(Method::POST, "/items")
        .parameter(Parameter::query("limit", Schema::integer()))
        .body(item_schema());
    let server = RunningServer::start(one_route(description, answering_called())?).await?;

    let response = post_json(
        &server,
        "/items?limit=many",
        r#"{"price": "free", "tags": [1]}"#,
    )
    .await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        json_body(response).await?,
        json!({"detail": [
            {
                "type": "int_parsing",
                "loc": ["query", "limit"],
                "msg": "Input should be a valid integer, unable to parse string as an integer",
                "input": "many",
            },
            {
                "type": "missing",
                "loc": ["body", "name"],
                "msg": "Field required",
                "input": {"price": "free", "tags": [1]},
            },
            {
                "type": "float_parsing",
                "loc": ["body", "price"],
                "msg": "Input should be a valid number, unable to parse string as a number",
                "input": "free",
            },
            {
                "type": "string_type",
                "loc": ["body", "tags", 0],
                "msg": "Input should be a valid string",
                "input": 1,
            },
        ]})
    );
    server.stop().await
}

#[tokio::test]
async fn returns_422_when_the_body_is_not_valid_json() -> TestResult {
    let description = RouteDescription::new(Method::POST, "/items").body(item_schema());
    let server = RunningServer::start(one_route(description, answering_called())?).await?;

    let response = post_json(&server, "/items", r#"{"name": }"#).await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.text().await?,
        concat!(
            r#"{"detail":[{"type":"json_invalid","loc":["body",9],"msg":"JSON decode error","#,
            r#""input":{},"ctx":{"error":"expected value"}}]}"#
        )
    );
    server.stop().await
}

#[tokio::test]
async fn returns_422_when_a_required_body_is_missing() -> TestResult {
    let description = RouteDescription::new(Method::POST, "/items").body(item_schema());
    let server = RunningServer::start(one_route(description, answering_called())?).await?;

    let response = reqwest::Client::new()
        .post(server.url("/items"))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.text().await?,
        r#"{"detail":[{"type":"missing","loc":["body"],"msg":"Field required","input":null}]}"#
    );
    server.stop().await
}

#[test]
fn description_errors_name_the_route_and_the_parameter() {
    let object = Schema::object([Field::new("a", Schema::string())]);
    let descriptions = [
        RouteDescription::new(Method::GET, "/items/{id}")
            .parameter(Parameter::path("item_id", Schema::integer())),
        RouteDescription::new(Method::GET, "/items/{id}")
            .parameter(Parameter::path("id", Schema::integer()).optional()),
        RouteDescription::new(Method::GET, "/items/{id}")
            .parameter(Parameter::path("id", Schema::array(Schema::integer()))),
        RouteDescription::new(Method::GET, "/items").parameter(Parameter::query("filter", object)),
        RouteDescription::new(Method::GET, "/items")
            .parameter(Parameter::query("q", Schema::string()))
            .parameter(Parameter::query("q", Schema::string())),
        RouteDescription::new(Method::GET, "/items/{id}"),
        RouteDescription::new(Method::POST, "/items").body(Schema::object([
            Field::new("name", Schema::string()),
            Field::new("name", Schema::string()),
        ])),
    ];

    let messages = descriptions.map(|description| {
        one_route(description, answering_called())
            .err()
            .map(|error| error.to_string())
    });

    assert_eq!(
        messages,
        [
            Some("path `/items/{id}` has no segment `{item_id}` for its path parameter `item_id`".to_owned()),
            Some("path parameter `id` of `/items/{id}` cannot be optional or have a default".to_owned()),
            Some("path parameter `id` of `/items/{id}` must be a string, integer, number or boolean".to_owned()),
            Some(
                "query parameter `filter` of `/items` must be a string, integer, number or boolean, or a list of those"
                    .to_owned()
            ),
            Some("query parameter `q` of `/items` is declared more than once".to_owned()),
            Some("path `/items/{id}` has the segment `{id}` but no path parameter `id`".to_owned()),
            Some("body of `/items` has the field `name` more than once in one object".to_owned()),
        ]
    );
}
