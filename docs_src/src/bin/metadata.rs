// ANCHOR: imports
use std::error::Error;

use thermite::{App, RouteError, Router, Server, get, routes};
// ANCHOR_END: imports

// ANCHOR: app
#[get("/users")]
async fn read_users() -> &'static str {
    "the users"
}

#[get("/items")]
async fn read_items() -> &'static str {
    "the items"
}

fn app() -> Result<App, RouteError> {
    let users = Router::new().tag("users").routes(routes![read_users]);
    let items = Router::new().tag("items").routes(routes![read_items]);
    App::new()
        .title("ChimichangApp")
        .version("0.0.1")
        .openapi_path("/api/v1/openapi.json")
        .docs_path("/documentation")
        .include_router(users)?
        .include_router(items)
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
    use thermite::{StatusCode, TestClient};

    use super::*;

    #[tokio::test]
    async fn serves_the_docs_at_the_paths_given() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let docs = client.get("/documentation").send().await?;
        let schema = client.get("/api/v1/openapi.json").send().await?;
        let old_docs = client.get("/docs").send().await?;
        let users = client.get("/users").send().await?;
        let items = client.get("/items").send().await?;

        assert_eq!(docs.status(), StatusCode::OK);
        assert_eq!(schema.status(), StatusCode::OK);
        assert_eq!(old_docs.status(), StatusCode::NOT_FOUND);
        assert_eq!(users.text()?, "the users");
        assert_eq!(items.text()?, "the items");
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers and parts of the schema as they are
#[cfg(test)]
mod shown_tests {
    use serde_json::json;
    use thermite::{StatusCode, TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn the_schema_carries_the_title_and_version() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/api/v1/openapi.json").send().await?.json()?;
        let docs = client.get("/documentation").send().await?;

        assert_eq!(
            openapi["info"],
            json!({"title": "ChimichangApp", "version": "0.0.1"})
        );
        assert_eq!(openapi["paths"]["/users"]["get"]["tags"], json!(["users"]));
        let page = docs.text()?;
        assert!(page.contains("<title>ChimichangApp - Swagger UI</title>"));
        assert!(page.contains("/api/v1/openapi.json"));
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_can_be_turned_off() -> Result<(), Box<dyn Error>> {
        let app = App::new().without_docs().routes(routes![read_users])?;
        let hidden = App::new().without_openapi().routes(routes![read_users])?;
        let client = TestClient::start(app).await?;
        let hidden_client = TestClient::start(hidden).await?;

        let docs = client.get("/docs").send().await?;
        let schema: Value = client.get("/openapi.json").send().await?.json()?;
        let hidden_docs = hidden_client.get("/docs").send().await?;
        let hidden_schema = hidden_client.get("/openapi.json").send().await?;

        assert_eq!(docs.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            schema["info"],
            json!({"title": "Thermite", "version": "0.1.0"})
        );
        assert_eq!(hidden_docs.status(), StatusCode::NOT_FOUND);
        assert_eq!(hidden_schema.status(), StatusCode::NOT_FOUND);
        client.stop().await;
        hidden_client.stop().await;
        Ok(())
    }
}
