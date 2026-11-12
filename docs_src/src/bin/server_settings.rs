// ANCHOR: imports
use std::error::Error;
use std::future::pending;
use std::time::Duration;

use serde::Deserialize;
use thermite::{App, Json, Model, RouteError, Server, get, post, routes};
// ANCHOR_END: imports

// ANCHOR: app
#[derive(Deserialize, Model)]
struct Item {
    name: String,
}

#[post("/items")]
async fn create_item(Json(item): Json<Item>) -> String {
    format!("created {}", item.name)
}

#[get("/slow")]
async fn slow() -> &'static str {
    tokio::time::sleep(Duration::from_millis(200)).await;
    "done"
}

fn app() -> Result<App, RouteError> {
    App::new().routes(routes![create_item, slow])
}
// ANCHOR_END: app

// ANCHOR: settings
fn configure(server: Server) -> Server {
    server
        .with_body_limit(10 * 1024 * 1024)
        .with_header_read_timeout(Duration::from_secs(10))
        .with_shutdown_timeout(Duration::from_secs(20))
}
// ANCHOR_END: settings

// ANCHOR: main
async fn ctrl_c() {
    if tokio::signal::ctrl_c().await.is_err() {
        // without a way to hear Ctrl+C, only killing the process stops the server
        pending::<()>().await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = configure(Server::bind("127.0.0.1:8000").await?);
    server.serve_with_shutdown(app()?, ctrl_c).await?;
    Ok(())
}
// ANCHOR_END: main

// ANCHOR: test
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use thermite::{Next, Request, StatusCode};
    use tokio::sync::Notify;

    use super::*;

    #[tokio::test]
    async fn finishes_the_requests_it_has_started_before_stopping() -> Result<(), Box<dyn Error>> {
        // small enough that the whole body is sent before the 413 arrives
        let server = configure(Server::bind("127.0.0.1:0").await?).with_body_limit(16);
        let url = format!("http://{}", server.local_address()?);
        let stop = Arc::new(Notify::new());
        let stop_requested = Arc::clone(&stop);
        let started = Arc::new(Notify::new());
        let reached_the_app = Arc::clone(&started);
        let app = app()?.middleware(move |request: Request, next: Next| {
            reached_the_app.notify_one();
            next.run(request)
        });
        let serving = tokio::spawn(server.serve_with_shutdown(app, move || {
            let stop = Arc::clone(&stop_requested);
            async move { stop.notified().await }
        }));

        let too_large = reqwest::Client::new()
            .post(format!("{url}/items"))
            .body(r#"{"name": "a long name"}"#)
            .send()
            .await?;
        let slow = tokio::spawn(reqwest::get(format!("{url}/slow")));
        started.notified().await;
        stop.notify_one();
        let slow = slow.await??;

        assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(too_large.text().await?, r#"{"detail":"Content Too Large"}"#);
        assert_eq!(slow.text().await?, "done");
        serving.await??;
        Ok(())
    }
}
// ANCHOR_END: test
