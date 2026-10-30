use thermite::http::header::AUTHORIZATION;
use thermite::http::{HeaderName, HeaderValue};
use thermite::{App, IntoResponse, Middleware, Next, Request, Response, StatusCode};

use crate::support::{RunningServer, TestResult};

const SEEN_BY: HeaderName = HeaderName::from_static("x-seen-by");

async fn mark_response(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&format!("middleware at {path}")) {
        response.headers_mut().insert(SEEN_BY, value);
    }
    response
}

#[tokio::test]
async fn a_function_middleware_wraps_routes_404s_and_the_docs() -> TestResult {
    let app = App::new()
        .middleware(mark_response)
        .get("/items", || async { "items" })?;
    let server = RunningServer::start(app).await?;

    for (path, status) in [
        ("/items", StatusCode::OK),
        ("/users", StatusCode::NOT_FOUND),
        ("/openapi.json", StatusCode::OK),
    ] {
        let response = reqwest::get(server.url(path)).await?;

        assert_eq!(response.status(), status, "{path}");
        assert_eq!(
            response.headers()[SEEN_BY],
            format!("middleware at {path}"),
            "{path}"
        );
    }
    server.stop().await
}

struct RequireToken {
    token: &'static str,
}

impl Middleware for RequireToken {
    async fn call(&self, request: Request, next: Next) -> Response {
        let expected = format!("Bearer {}", self.token);
        match request.headers().get(AUTHORIZATION) {
            Some(value) if *value == *expected => next.run(request).await,
            _ => StatusCode::UNAUTHORIZED.into_response(),
        }
    }
}

#[tokio::test]
async fn a_middleware_struct_can_answer_before_the_route_runs() -> TestResult {
    let app = App::new()
        .middleware(RequireToken { token: "secret" })
        .get("/items", || async { "items" })?;
    let server = RunningServer::start(app).await?;
    let client = reqwest::Client::new();

    let refused = client.get(server.url("/items")).send().await?;
    let allowed = client
        .get(server.url("/items"))
        .header(AUTHORIZATION, "Bearer secret")
        .send()
        .await?;

    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(allowed.text().await?, "items");
    server.stop().await
}

#[tokio::test]
async fn closures_run_the_middleware_added_last_first() -> TestResult {
    let app = App::new()
        .middleware(|request: Request, next: Next| async move {
            let mut response = next.run(request).await;
            response
                .headers_mut()
                .append(SEEN_BY, HeaderValue::from_static("first added"));
            response
        })
        .middleware(|request: Request, next: Next| async move {
            let mut response = next.run(request).await;
            response
                .headers_mut()
                .append(SEEN_BY, HeaderValue::from_static("last added"));
            response
        })
        .get("/", || async {})?;
    let server = RunningServer::start(app).await?;

    let response = reqwest::get(server.url("/")).await?;
    let seen: Vec<_> = response.headers().get_all(SEEN_BY).iter().collect();

    assert_eq!(seen, ["first added", "last added"]);
    server.stop().await
}
