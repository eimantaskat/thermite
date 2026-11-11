// ANCHOR: imports
use std::error::Error;
use std::time::Instant;

use thermite::http::HeaderValue;
use thermite::{App, GZipMiddleware, Next, Request, Response, RouteError, Server, get, routes};
// ANCHOR_END: imports

// ANCHOR: process_time
async fn add_process_time(request: Request, next: Next) -> Response {
    let start = Instant::now();
    let mut response = next.run(request).await;
    let seconds = start.elapsed().as_secs_f64();
    if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
        response.headers_mut().insert("x-process-time", value);
    }
    response
}
// ANCHOR_END: process_time

// ANCHOR: routes
#[get("/")]
async fn root() -> &'static str {
    "Hello World"
}

#[get("/report")]
async fn report() -> String {
    "a long line of the report\n".repeat(100)
}
// ANCHOR_END: routes

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new()
        .middleware(add_process_time)
        .middleware(GZipMiddleware::new().minimum_size(1000))
        .routes(routes![root, report])
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
    async fn times_every_response() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        for path in ["/", "/missing"] {
            let response = client.get(path).send().await?;
            let seconds: f64 = response.headers()["x-process-time"].to_str()?.parse()?;
            assert!(seconds >= 0.0, "{path}");
        }
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers as they are
#[cfg(test)]
mod shown_tests {
    use thermite::TestClient;
    use thermite::http::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, VARY};

    use super::*;

    #[tokio::test]
    async fn compresses_only_large_responses_for_clients_that_accept_it()
    -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let small = client
            .get("/")
            .header(ACCEPT_ENCODING, "gzip")
            .send()
            .await?;
        let large = client
            .get("/report")
            .header(ACCEPT_ENCODING, "gzip")
            .send()
            .await?;
        let plain = client.get("/report").send().await?;

        assert_eq!(small.text()?, "Hello World");
        assert!(small.headers().get(CONTENT_ENCODING).is_none());
        assert_eq!(large.headers()[CONTENT_ENCODING], "gzip");
        assert_eq!(large.headers()[VARY], "Accept-Encoding");
        assert!(large.body().len() < 2600);
        assert!(plain.headers().get(CONTENT_ENCODING).is_none());
        assert_eq!(plain.headers()[CONTENT_LENGTH], "2600");
        client.stop().await;
        Ok(())
    }
}
