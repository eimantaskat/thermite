// ANCHOR: imports
use std::error::Error;

use thermite::http::header::HOST;
use thermite::{
    App, IntoResponse, Middleware, Next, Request, Response, RouteError, Server, StatusCode, get,
    routes,
};
// ANCHOR_END: imports

// ANCHOR: middleware
struct TrustedHostMiddleware {
    allowed_hosts: Vec<String>,
}

impl TrustedHostMiddleware {
    fn new(allowed_hosts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allowed_hosts: allowed_hosts.into_iter().map(Into::into).collect(),
        }
    }

    fn allows(&self, host: &str) -> bool {
        self.allowed_hosts.iter().any(|allowed| {
            allowed == "*"
                || allowed == host
                || allowed
                    .strip_prefix('*')
                    .is_some_and(|suffix| suffix.starts_with('.') && host.ends_with(suffix))
        })
    }
}

impl Middleware for TrustedHostMiddleware {
    async fn call(&self, request: Request, next: Next) -> Response {
        let host = request
            .headers()
            .get(HOST)
            .and_then(|value| value.to_str().ok())
            .map(|host| host.split_once(':').map_or(host, |(name, _port)| name));
        match host {
            Some(host) if self.allows(host) => next.run(request).await,
            _ => (StatusCode::BAD_REQUEST, "Invalid host header").into_response(),
        }
    }
}
// ANCHOR_END: middleware

// ANCHOR: app
#[get("/")]
async fn main_page() -> &'static str {
    "Hello World"
}

fn app() -> Result<App, RouteError> {
    App::new()
        .middleware(TrustedHostMiddleware::new(["example.com", "*.example.com"]))
        .routes(routes![main_page])
}
// ANCHOR_END: app

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

    use super::*;

    #[tokio::test]
    async fn answers_only_the_hosts_it_trusts() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let mut answers = Vec::new();
        for host in [
            "example.com",
            "api.example.com:8000",
            "evilexample.com",
            "testserver",
        ] {
            let response = client.get("/").header(HOST, host).send().await?;
            answers.push((response.status(), response.text()?.to_owned()));
        }

        let invalid = (StatusCode::BAD_REQUEST, "Invalid host header".to_owned());
        let answered = (StatusCode::OK, "Hello World".to_owned());
        assert_eq!(
            answers,
            [answered.clone(), answered, invalid.clone(), invalid]
        );
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test
