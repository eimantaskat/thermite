// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{
    App, Header, HttpError, Json, Model, Path, RouteError, Server, StatusCode, get, post, routes,
};
// ANCHOR_END: imports

// ANCHOR: app
#[derive(Deserialize, Model)]
struct TokenHeader {
    x_token: String,
}

#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: String,
}

#[derive(Serialize, Deserialize, Model)]
struct Item {
    id: String,
    title: String,
    description: Option<String>,
}

fn check_token(header: &TokenHeader) -> Result<(), HttpError> {
    if header.x_token == "coneofsilence" {
        Ok(())
    } else {
        Err(HttpError::new(
            StatusCode::BAD_REQUEST,
            "Invalid X-Token header",
        ))
    }
}

#[get("/items/{item_id}")]
async fn read_item(
    Path(ItemPath { item_id }): Path<ItemPath>,
    Header(header): Header<TokenHeader>,
) -> Result<Json<Item>, HttpError> {
    check_token(&header)?;
    if item_id != "foo" {
        return Err(HttpError::new(StatusCode::NOT_FOUND, "Item not found"));
    }
    Ok(Json(Item {
        id: item_id,
        title: "Foo".to_owned(),
        description: Some("There goes my hero".to_owned()),
    }))
}

#[post("/items")]
async fn create_item(
    Header(header): Header<TokenHeader>,
    Json(item): Json<Item>,
) -> Result<Json<Item>, HttpError> {
    check_token(&header)?;
    Ok(Json(item))
}

fn app() -> Result<App, RouteError> {
    App::new().routes(routes![read_item, create_item])
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
    use thermite::{TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn reads_an_item() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .get("/items/foo")
            .header("x-token", "coneofsilence")
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.text()?,
            r#"{"id":"foo","title":"Foo","description":"There goes my hero"}"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn refuses_a_bad_token() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .get("/items/foo")
            .header("x-token", "hailhydra")
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: Value = response.json()?;
        assert_eq!(body["detail"], "Invalid X-Token header");
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn answers_404_for_an_unknown_item() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .get("/items/bar")
            .header("x-token", "coneofsilence")
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.text()?, r#"{"detail":"Item not found"}"#);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn creates_an_item() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let item = Item {
            id: "foobar".to_owned(),
            title: "Foo Bar".to_owned(),
            description: None,
        };

        let response = client
            .post("/items")
            .header("x-token", "coneofsilence")
            .json(&item)
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let created: Item = response.json()?;
        assert_eq!(created.title, "Foo Bar");
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test
