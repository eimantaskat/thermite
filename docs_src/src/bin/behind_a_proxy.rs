// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{App, Header, Json, Model, RouteError, Router, Server, get, routes};
// ANCHOR_END: imports

// ANCHOR: app
#[derive(Deserialize, Model)]
struct ForwardedHeaders {
    x_forwarded_for: Option<String>,
    x_forwarded_proto: Option<String>,
}

#[derive(Serialize, Model)]
struct Client {
    client: Option<String>,
    scheme: String,
}

#[get("/client")]
async fn read_client(Header(forwarded): Header<ForwardedHeaders>) -> Json<Client> {
    // the gateway appends the address it saw, so the last one is the only one it vouches for
    let client = forwarded
        .x_forwarded_for
        .as_deref()
        .and_then(|addresses| addresses.rsplit(',').next())
        .map(|address| address.trim().to_owned());
    Json(Client {
        client,
        scheme: forwarded
            .x_forwarded_proto
            .unwrap_or_else(|| "http".to_owned()),
    })
}

fn app() -> Result<App, RouteError> {
    let api = Router::new().prefix("/api/v1").routes(routes![read_client]);
    App::new()
        .openapi_path("/api/v1/openapi.json")
        .docs_path("/api/v1/docs")
        .include_router(api)
}
// ANCHOR_END: app

// ANCHOR: main
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = Server::bind("0.0.0.0:8000")
        .await?
        .with_header_read_timeout(None);
    server.serve(app()?).await?;
    Ok(())
}
// ANCHOR_END: main

// ANCHOR: test
#[cfg(test)]
mod tests {
    use thermite::{StatusCode, TestClient};

    use super::*;

    #[tokio::test]
    async fn reads_the_client_the_gateway_forwarded() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let forwarded = client
            .get("/api/v1/client")
            .header("x-forwarded-for", "203.0.113.7, 198.51.100.2")
            .header("x-forwarded-proto", "https")
            .send()
            .await?;
        let direct = client.get("/api/v1/client").send().await?;
        let docs = client.get("/api/v1/docs").send().await?;

        assert_eq!(
            forwarded.text()?,
            r#"{"client":"198.51.100.2","scheme":"https"}"#
        );
        assert_eq!(direct.text()?, r#"{"client":null,"scheme":"http"}"#);
        assert_eq!(docs.status(), StatusCode::OK);
        assert!(docs.text()?.contains("/api/v1/openapi.json"));
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these paths as they are
#[cfg(test)]
mod shown_tests {
    use thermite::{StatusCode, TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn serves_everything_under_the_prefix() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/api/v1/openapi.json").send().await?.json()?;
        let stripped = client.get("/client").send().await?;

        let paths: Vec<_> = openapi["paths"]
            .as_object()
            .map(|paths| paths.keys().cloned().collect())
            .unwrap_or_default();
        assert_eq!(paths, ["/api/v1/client"]);
        assert_eq!(stripped.status(), StatusCode::NOT_FOUND);
        client.stop().await;
        Ok(())
    }
}
