// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{
    App, Json, Model, Path, RequestValidationError, RouteError, Server, StatusCode, get, routes,
};
// ANCHOR_END: imports

#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: i64,
}

#[derive(Serialize, Model)]
struct Item {
    item_id: i64,
}

#[get("/items/{item_id}")]
async fn read_item(Path(ItemPath { item_id }): Path<ItemPath>) -> Json<Item> {
    Json(Item { item_id })
}

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new()
        .exception_handler(|error: &RequestValidationError| {
            let lines: Vec<String> = error
                .errors()
                .iter()
                .map(|problem| {
                    let message = problem["msg"].as_str().unwrap_or_default();
                    format!("{}: {message}", problem["loc"])
                })
                .collect();
            (StatusCode::BAD_REQUEST, lines.join("\n"))
        })
        .routes(routes![read_item])
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
    async fn answers_invalid_requests_in_plain_text() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/items/foo").send().await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.text()?,
            r#"["path","item_id"]: Input should be a valid integer, unable to parse string as an integer"#
        );
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page says the docs list no answer for an invalid request
#[cfg(test)]
mod shown_tests {
    use thermite::{TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn the_docs_no_longer_list_a_422() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        let responses = &openapi["paths"]["/items/{item_id}"]["get"]["responses"];
        let statuses: Vec<_> = responses
            .as_object()
            .into_iter()
            .flat_map(|responses| responses.keys())
            .collect();
        assert_eq!(statuses, ["200"]);
        client.stop().await;
        Ok(())
    }
}
