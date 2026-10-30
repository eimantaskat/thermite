use bytes::Bytes;
use http::{Method, StatusCode};
use thermite_core::{
    Arguments, Parameter, Request, Response, RouteDescription, RouteError, RouteHandler, Router,
    Schema,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::support::{RunningServer, TEST_TIMEOUT, TestResult, one_route, text_response};

fn answering(body: &'static str) -> impl RouteHandler {
    move |_request: Request, _arguments: Arguments| async move { text_response(StatusCode::OK, body) }
}

fn echoing_parameter(name: &'static str) -> impl RouteHandler {
    move |_request: Request, arguments: Arguments| async move {
        let value = arguments
            .path(name)
            .and_then(|value| value.as_str())
            .unwrap_or("<missing>")
            .to_owned();
        Response::new(Bytes::from(value))
    }
}

fn described(method: Method, path: &str) -> RouteDescription {
    RouteDescription::new(method, path)
}

fn with_text_parameter(method: Method, path: &str, name: &str) -> RouteDescription {
    RouteDescription::new(method, path).parameter(Parameter::path(name, Schema::string()))
}

#[tokio::test]
async fn dispatches_requests_by_method_and_path() -> TestResult {
    let mut router = Router::new();
    router.route(described(Method::GET, "/items"), answering("list"))?;
    router.route(described(Method::POST, "/items"), answering("create"))?;
    router.route(described(Method::GET, "/users"), answering("users"))?;
    let server = RunningServer::start(router).await?;
    let client = reqwest::Client::new();

    let listed = client.get(server.url("/items")).send().await?;
    let created = client.post(server.url("/items")).send().await?;
    let users = client.get(server.url("/users?active=true")).send().await?;

    assert_eq!(listed.text().await?, "list");
    assert_eq!(created.text().await?, "create");
    assert_eq!(users.text().await?, "users");
    server.stop().await
}

#[tokio::test]
async fn passes_decoded_path_parameters_to_the_handler() -> TestResult {
    let router = one_route(
        with_text_parameter(Method::GET, "/items/{item_id}", "item_id"),
        echoing_parameter("item_id"),
    )?;
    let server = RunningServer::start(router).await?;

    let response = reqwest::get(server.url("/items/hello%20world")).await?;

    assert_eq!(response.text().await?, "hello world");
    server.stop().await
}

#[tokio::test]
async fn returns_404_in_fastapi_format_when_no_route_matches() -> TestResult {
    let router = one_route(described(Method::GET, "/items"), answering("list"))?;
    let server = RunningServer::start(router).await?;

    for path in ["/users", "/items/", "/items/5"] {
        let response = reqwest::get(server.url(path)).await?;

        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.text().await?, r#"{"detail":"Not Found"}"#);
    }
    server.stop().await
}

#[tokio::test]
async fn returns_405_with_the_allowed_methods_when_only_the_method_differs() -> TestResult {
    let mut router = Router::new();
    router.route(described(Method::GET, "/items"), answering("list"))?;
    router.route(described(Method::POST, "/items"), answering("create"))?;
    let server = RunningServer::start(router).await?;

    let response = reqwest::Client::new()
        .delete(server.url("/items"))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()["allow"], "GET, HEAD, POST");
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.text().await?, r#"{"detail":"Method Not Allowed"}"#);
    server.stop().await
}

#[tokio::test]
async fn a_literal_route_wins_over_a_parameter_route_registered_before_it() -> TestResult {
    let mut router = Router::new();
    router.route(
        with_text_parameter(Method::GET, "/users/{user_id}", "user_id"),
        echoing_parameter("user_id"),
    )?;
    router.route(
        described(Method::GET, "/users/me"),
        answering("current user"),
    )?;
    let server = RunningServer::start(router).await?;

    let me = reqwest::get(server.url("/users/me")).await?;
    let other = reqwest::get(server.url("/users/42")).await?;

    assert_eq!(me.text().await?, "current user");
    assert_eq!(other.text().await?, "42");
    server.stop().await
}

#[tokio::test]
async fn falls_back_to_a_parameter_route_that_accepts_the_method() -> TestResult {
    let mut router = Router::new();
    router.route(
        described(Method::GET, "/users/me"),
        answering("current user"),
    )?;
    router.route(
        with_text_parameter(Method::DELETE, "/users/{user_id}", "user_id"),
        echoing_parameter("user_id"),
    )?;
    let server = RunningServer::start(router).await?;

    let response = reqwest::Client::new()
        .delete(server.url("/users/me"))
        .send()
        .await?;

    assert_eq!(response.text().await?, "me");
    server.stop().await
}

#[tokio::test]
async fn answers_head_from_a_get_route_without_sending_the_body() -> TestResult {
    let router = one_route(described(Method::GET, "/items"), answering("list"))?;
    let server = RunningServer::start(router).await?;

    let mut stream = TcpStream::connect(server.address).await?;
    stream
        .write_all(b"HEAD /items HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n")
        .await?;
    let mut response = String::new();
    timeout(TEST_TIMEOUT, stream.read_to_string(&mut response)).await??;

    assert!(
        response.starts_with("HTTP/1.1 200"),
        "unexpected response: {response}"
    );
    assert!(
        response.ends_with("\r\n\r\n"),
        "HEAD response has a body: {response}"
    );
    server.stop().await
}

#[test]
fn rejects_a_route_matching_the_same_requests_as_an_existing_one() -> TestResult {
    let mut router = Router::new();
    router.route(
        with_text_parameter(Method::GET, "/items/{item_id}", "item_id"),
        answering("get"),
    )?;
    router.route(
        with_text_parameter(Method::DELETE, "/items/{name}", "name"),
        answering("delete"),
    )?;

    let error = router
        .route(
            with_text_parameter(Method::GET, "/items/{name}", "name"),
            answering("again"),
        )
        .err();

    assert_eq!(
        error,
        Some(RouteError::Conflict {
            method: Method::GET,
            path: "/items/{name}".to_owned(),
            existing: "/items/{item_id}".to_owned(),
        })
    );
    Ok(())
}

#[tokio::test]
async fn keeps_serving_its_routes_after_one_fails_to_register() -> TestResult {
    let mut router = Router::new();
    router.route(described(Method::GET, "/items"), answering("list"))?;

    let failed = router.route(described(Method::GET, "/items"), answering("never"));
    let server = RunningServer::start(router).await?;
    let response = reqwest::get(server.url("/items")).await?;

    assert!(matches!(failed, Err(RouteError::Conflict { .. })));
    assert_eq!(response.text().await?, "list");
    server.stop().await
}

#[tokio::test]
async fn serves_a_prefixed_route_with_the_path_parameters_of_prefix_and_route() -> TestResult {
    let description = RouteDescription::new(Method::GET, "/items/{item_id}")
        .with_prefix("/{user_id}")
        .with_prefix("/users")
        .parameter(Parameter::path("user_id", Schema::string()))
        .parameter(Parameter::path("item_id", Schema::string()));
    let router = one_route(
        description,
        |_request: Request, arguments: Arguments| async move {
            let value = |name| {
                arguments
                    .path(name)
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            };
            Response::new(Bytes::from(format!(
                "{} {}",
                value("user_id"),
                value("item_id")
            )))
        },
    )?;
    let server = RunningServer::start(router).await?;

    let response = reqwest::get(server.url("/users/ada/items/7")).await?;

    assert_eq!(response.text().await?, "ada 7");
    server.stop().await
}

#[test]
fn route_errors_name_the_offending_path() {
    let messages = ["items", "/items/{id", "/{id}/{id}"].map(|path| {
        one_route(described(Method::GET, path), answering(""))
            .err()
            .map(|error| error.to_string())
    });
    let conflict = one_route(described(Method::GET, "/items"), answering(""))
        .and_then(|mut router| router.route(described(Method::GET, "/items"), answering("")))
        .err()
        .map(|error| error.to_string());
    let prefixes = ["users", "/users/", "/"].map(|prefix| {
        one_route(
            described(Method::GET, "/items").with_prefix(prefix),
            answering(""),
        )
        .err()
        .map(|error| error.to_string())
    });

    assert_eq!(
        messages,
        [
            Some("path `items` must start with `/`".to_owned()),
            Some("path `/items/{id` has an invalid parameter segment `{id`".to_owned()),
            Some("path `/{id}/{id}` uses the parameter `id` more than once".to_owned()),
        ]
    );
    assert_eq!(
        conflict.as_deref(),
        Some("GET `/items` matches the same requests as the route `/items`")
    );
    assert_eq!(
        prefixes,
        [
            Some(
                "prefix `users` of `users/items` must start with `/` and must not end with `/`"
                    .to_owned()
            ),
            Some(
                "prefix `/users/` of `/users//items` must start with `/` and must not end with `/`"
                    .to_owned()
            ),
            Some(
                "prefix `/` of `//items` must start with `/` and must not end with `/`".to_owned()
            ),
        ]
    );
}
