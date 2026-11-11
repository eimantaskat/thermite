use std::error::Error;
use std::time::Duration;

use thermite::http::HeaderName;
use thermite::http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
    ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, ALLOW, CONTENT_TYPE, ORIGIN,
    VARY,
};
use thermite::{App, CorsMiddleware, GZipMiddleware, Method, StatusCode, TestClient, TestRequest};

use crate::support::TestResult;

const EXAMPLE: &str = "https://example.org";
const PREFLIGHT_VARY: &str = "Origin, Access-Control-Request-Method, Access-Control-Request-Headers, Access-Control-Request-Private-Network";
const REQUEST_PRIVATE_NETWORK: &str = "access-control-request-private-network";
const ALLOW_PRIVATE_NETWORK: &str = "access-control-allow-private-network";

async fn client(cors: CorsMiddleware) -> Result<TestClient, Box<dyn Error>> {
    let app = App::new()
        .middleware(cors)
        .get("/items", || async { "items" })?;
    Ok(TestClient::start(app).await?)
}

fn preflight<'client>(client: &'client TestClient, origin: &str) -> TestRequest<'client> {
    client
        .options("/items")
        .header(ORIGIN, origin)
        .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
}

#[tokio::test]
async fn allowing_everything_with_credentials_mirrors_the_origin() -> TestResult {
    let client = client(
        CorsMiddleware::new()
            .allow_origins(["*"])
            .allow_any_method()
            .allow_any_header()
            .allow_credentials(true)
            .expose_headers([HeaderName::from_static("x-status")]),
    )
    .await?;

    let answered = preflight(&client, EXAMPLE)
        .header(ACCESS_CONTROL_REQUEST_HEADERS, "X-Example, X-Other")
        .send()
        .await?;
    let simple = client.get("/items").header(ORIGIN, EXAMPLE).send().await?;
    let without_origin = client.get("/items").send().await?;

    assert_eq!(answered.status(), StatusCode::OK);
    assert_eq!(answered.text()?, "OK");
    assert_eq!(
        answered.headers()[CONTENT_TYPE],
        "text/plain; charset=utf-8"
    );
    assert_eq!(answered.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], EXAMPLE);
    assert_eq!(
        answered.headers()[ACCESS_CONTROL_ALLOW_HEADERS],
        "X-Example, X-Other"
    );
    assert_eq!(
        answered.headers()[ACCESS_CONTROL_ALLOW_METHODS],
        "DELETE, GET, HEAD, OPTIONS, PATCH, POST, PUT, QUERY"
    );
    assert_eq!(answered.headers()[ACCESS_CONTROL_ALLOW_CREDENTIALS], "true");
    assert_eq!(answered.headers()[ACCESS_CONTROL_MAX_AGE], "600");
    assert_eq!(answered.headers()[VARY], PREFLIGHT_VARY);

    assert_eq!(simple.text()?, "items");
    assert_eq!(simple.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], EXAMPLE);
    assert_eq!(simple.headers()[ACCESS_CONTROL_ALLOW_CREDENTIALS], "true");
    assert_eq!(simple.headers()[ACCESS_CONTROL_EXPOSE_HEADERS], "x-status");
    assert_eq!(simple.headers()[VARY], "Origin");

    assert!(
        !without_origin
            .headers()
            .contains_key(ACCESS_CONTROL_ALLOW_ORIGIN)
    );
    assert_eq!(without_origin.headers()[VARY], "Origin");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn allowing_any_origin_without_credentials_answers_a_star() -> TestResult {
    let client = client(CorsMiddleware::new().allow_origins(["*"])).await?;

    let answered = preflight(&client, EXAMPLE).send().await?;
    let simple = client.get("/items").header(ORIGIN, EXAMPLE).send().await?;

    assert_eq!(answered.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert_eq!(answered.headers()[ACCESS_CONTROL_ALLOW_METHODS], "GET");
    assert_eq!(
        answered.headers()[ACCESS_CONTROL_ALLOW_HEADERS],
        "Accept, Accept-Language, Content-Language, Content-Type"
    );
    assert!(
        !answered
            .headers()
            .contains_key(ACCESS_CONTROL_ALLOW_CREDENTIALS)
    );
    assert_eq!(simple.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert_eq!(simple.headers()[VARY], "Origin");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn allows_a_listed_origin_with_the_listed_methods_and_headers() -> TestResult {
    let client = client(
        CorsMiddleware::new()
            .allow_origins([EXAMPLE])
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([HeaderName::from_static("x-example")])
            .max_age(Duration::from_secs(60)),
    )
    .await?;

    let answered = preflight(&client, EXAMPLE)
        .header(ACCESS_CONTROL_REQUEST_HEADERS, "X-Example, content-type")
        .send()
        .await?;
    let simple = client.get("/items").header(ORIGIN, EXAMPLE).send().await?;
    let other = client
        .get("/items")
        .header(ORIGIN, "https://other.org")
        .send()
        .await?;

    assert_eq!(answered.status(), StatusCode::OK);
    assert_eq!(answered.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], EXAMPLE);
    assert_eq!(
        answered.headers()[ACCESS_CONTROL_ALLOW_METHODS],
        "GET, POST"
    );
    assert_eq!(
        answered.headers()[ACCESS_CONTROL_ALLOW_HEADERS],
        "Accept, Accept-Language, Content-Language, Content-Type, x-example"
    );
    assert_eq!(answered.headers()[ACCESS_CONTROL_MAX_AGE], "60");
    assert_eq!(simple.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], EXAMPLE);
    assert_eq!(simple.headers()[VARY], "Origin");
    assert_eq!(other.text()?, "items");
    assert!(!other.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
    assert_eq!(other.headers()[VARY], "Origin");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn sends_the_credentials_header_but_no_origin_to_an_origin_not_listed() -> TestResult {
    let client = client(
        CorsMiddleware::new()
            .allow_origins([EXAMPLE])
            .allow_credentials(true)
            .expose_headers([HeaderName::from_static("x-status")]),
    )
    .await?;

    let response = client
        .get("/items")
        .header(ORIGIN, "https://other.org")
        .send()
        .await?;

    assert!(!response.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
    assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_CREDENTIALS], "true");
    assert_eq!(
        response.headers()[ACCESS_CONTROL_EXPOSE_HEADERS],
        "x-status"
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn refuses_a_preflight_naming_each_part_that_is_not_allowed() -> TestResult {
    let client = client(CorsMiddleware::new().allow_origins([EXAMPLE])).await?;

    let refused = client
        .options("/items")
        .header(ORIGIN, "https://other.org")
        .header(ACCESS_CONTROL_REQUEST_METHOD, "DELETE")
        .header(ACCESS_CONTROL_REQUEST_HEADERS, "accept, x-secret")
        .header(REQUEST_PRIVATE_NETWORK, "true")
        .send()
        .await?;
    let empty_header_name = preflight(&client, EXAMPLE)
        .header(ACCESS_CONTROL_REQUEST_HEADERS, "accept,")
        .send()
        .await?;

    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        refused.text()?,
        "Disallowed CORS origin, method, headers, private-network"
    );
    assert!(!refused.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
    assert_eq!(refused.headers()[VARY], PREFLIGHT_VARY);
    assert_eq!(empty_header_name.text()?, "Disallowed CORS headers");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn a_predicate_allows_a_family_of_origins_beside_the_listed_ones() -> TestResult {
    let client = client(
        CorsMiddleware::new()
            .allow_origins(["https://partner.org"])
            .allow_origin_fn(|origin| {
                origin
                    .strip_prefix("https://")
                    .is_some_and(|host| host.ends_with(".example.org"))
            }),
    )
    .await?;

    for (origin, allowed) in [
        ("https://api.example.org", true),
        ("https://partner.org", true),
        ("https://example.org.evil.com", false),
        ("http://api.example.org", false),
    ] {
        let simple = client.get("/items").header(ORIGIN, origin).send().await?;
        let answered = preflight(&client, origin).send().await?;

        assert_eq!(
            simple.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_some(),
            allowed,
            "{origin}"
        );
        assert_eq!(answered.status().is_success(), allowed, "{origin}");
    }
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn allows_a_private_network_only_when_told_to() -> TestResult {
    let client = client(
        CorsMiddleware::new()
            .allow_origins([EXAMPLE])
            .allow_private_network(true),
    )
    .await?;

    let answered = preflight(&client, EXAMPLE)
        .header(REQUEST_PRIVATE_NETWORK, "true")
        .send()
        .await?;

    assert_eq!(answered.status(), StatusCode::OK);
    assert_eq!(answered.headers()[ALLOW_PRIVATE_NETWORK], "true");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_a_preflight_before_routing_and_lets_other_options_requests_through() -> TestResult
{
    let client = client(CorsMiddleware::new().allow_origins([EXAMPLE])).await?;

    let options = client
        .options("/items")
        .header(ORIGIN, EXAMPLE)
        .send()
        .await?;
    let preflight_of_404 = client
        .options("/missing")
        .header(ORIGIN, EXAMPLE)
        .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .send()
        .await?;

    assert_eq!(options.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(options.headers().contains_key(ALLOW));
    assert_eq!(options.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], EXAMPLE);
    assert_eq!(preflight_of_404.status(), StatusCode::OK);
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn adds_origin_to_the_vary_of_a_compressed_response() -> TestResult {
    let app = App::new()
        .middleware(GZipMiddleware::new())
        .middleware(CorsMiddleware::new().allow_origins([EXAMPLE]))
        .get("/items", || async { "x".repeat(1000) })?;
    let client = TestClient::start(app).await?;

    let response = client
        .get("/items")
        .header(ORIGIN, EXAMPLE)
        .header("accept-encoding", "gzip")
        .send()
        .await?;

    let values: Vec<_> = response.headers().get_all(VARY).iter().collect();
    assert_eq!(values, ["Accept-Encoding, Origin"]);
    client.stop().await;
    Ok(())
}
