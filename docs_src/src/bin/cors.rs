// ANCHOR: imports
use std::error::Error;

use thermite::{App, CorsMiddleware, RouteError, Server, get, routes};
// ANCHOR_END: imports

// ANCHOR: app
#[get("/")]
async fn root() -> &'static str {
    "Hello World"
}

fn app() -> Result<App, RouteError> {
    let cors = CorsMiddleware::new()
        .allow_origins(["http://localhost:8080", "https://example.com"])
        .allow_credentials(true)
        .allow_any_method()
        .allow_any_header();
    App::new().middleware(cors).routes(routes![root])
}
// ANCHOR_END: app

// shown on the page and used only by its test
#[cfg(test)]
// ANCHOR: origin_fn
fn subdomains() -> CorsMiddleware {
    CorsMiddleware::new().allow_origin_fn(|origin| origin.ends_with(".example.com"))
}
// ANCHOR_END: origin_fn

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = Server::bind("127.0.0.1:8000").await?;
    server.serve(app()?).await?;
    Ok(())
}

// ANCHOR: test
#[cfg(test)]
mod tests {
    use thermite::TestClient;
    use thermite::http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, ORIGIN};

    use super::*;

    #[tokio::test]
    async fn lets_a_listed_origin_read_the_answer() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let listed = client
            .get("/")
            .header(ORIGIN, "http://localhost:8080")
            .send()
            .await?;
        let other = client
            .get("/")
            .header(ORIGIN, "https://evil.example")
            .send()
            .await?;

        assert_eq!(
            listed.headers()[ACCESS_CONTROL_ALLOW_ORIGIN],
            "http://localhost:8080"
        );
        assert!(other.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers as they are
#[cfg(test)]
mod shown_tests {
    use thermite::http::header::{
        ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_METHOD, ORIGIN,
    };
    use thermite::{StatusCode, TestClient, TestResponse};

    use super::*;

    // the CORS headers as the page lists them, one per line
    fn cors_headers(response: &TestResponse) -> String {
        let mut lines: Vec<String> = response
            .headers()
            .iter()
            .filter(|(name, _)| name.as_str().starts_with("access-control-") || *name == "vary")
            .map(|(name, value)| format!("{name}: {}", value.to_str().unwrap_or_default()))
            .collect();
        lines.sort();
        lines.join("\n")
    }

    #[tokio::test]
    async fn answers_a_preflight() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let allowed = client
            .options("/")
            .header(ORIGIN, "https://example.com")
            .header(ACCESS_CONTROL_REQUEST_METHOD, "PUT")
            .send()
            .await?;
        let refused = client
            .options("/")
            .header(ORIGIN, "https://evil.example")
            .header(ACCESS_CONTROL_REQUEST_METHOD, "PUT")
            .send()
            .await?;
        let simple = client
            .get("/")
            .header(ORIGIN, "https://example.com")
            .send()
            .await?;

        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(allowed.text()?, "OK");
        assert_eq!(
            cors_headers(&allowed),
            concat!(
                "access-control-allow-credentials: true\n",
                "access-control-allow-methods: DELETE, GET, HEAD, OPTIONS, PATCH, POST, PUT, QUERY\n",
                "access-control-allow-origin: https://example.com\n",
                "access-control-max-age: 600\n",
                "vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers, ",
                "Access-Control-Request-Private-Network"
            )
        );
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        assert_eq!(refused.text()?, "Disallowed CORS origin");
        assert_eq!(
            cors_headers(&simple),
            concat!(
                "access-control-allow-credentials: true\n",
                "access-control-allow-origin: https://example.com\n",
                "vary: Origin"
            )
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_closure_can_decide_which_origins_are_allowed() -> Result<(), Box<dyn Error>> {
        let app = App::new().middleware(subdomains()).routes(routes![root])?;
        let client = TestClient::start(app).await?;

        let mut allowed = Vec::new();
        for origin in ["https://api.example.com", "https://evilexample.com"] {
            let response = client.get("/").header(ORIGIN, origin).send().await?;
            allowed.push(response.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
        }

        assert_eq!(allowed, [true, false]);
        client.stop().await;
        Ok(())
    }
}
