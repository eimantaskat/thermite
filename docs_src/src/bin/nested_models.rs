// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{App, Json, Model, RouteError, Server, post, routes};
// ANCHOR_END: imports

// ANCHOR: models
#[derive(Serialize, Deserialize, Model)]
struct Image {
    url: String,
    name: String,
}

#[derive(Serialize, Deserialize, Model)]
struct Item {
    name: String,
    price: f64,
    #[model(default)]
    tags: Vec<String>,
    image: Option<Image>,
}

#[derive(Serialize, Deserialize, Model)]
struct Offer {
    name: String,
    items: Vec<Item>,
}
// ANCHOR_END: models

// ANCHOR: handlers
#[post("/offers")]
async fn create_offer(Json(offer): Json<Offer>) -> Json<Offer> {
    Json(offer)
}

#[post("/images/multiple")]
async fn create_images(Json(images): Json<Vec<Image>>) -> Json<Vec<Image>> {
    Json(images)
}
// ANCHOR_END: handlers

// ANCHOR: comment
#[derive(Serialize, Deserialize, Model)]
struct Comment {
    text: String,
    replies: Vec<Comment>,
}

#[post("/comments")]
async fn create_comment(Json(comment): Json<Comment>) -> Json<Comment> {
    Json(comment)
}
// ANCHOR_END: comment

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![create_offer, create_images, create_comment])
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
    async fn reads_models_inside_models() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let body = r#"{
            "name": "Spring sale",
            "items": [
                {"name": "Foo", "price": 35.4, "image": {"url": "https://example.com/foo.png", "name": "Foo"}},
                {"name": "Bar", "price": 12.0, "tags": ["metal"]}
            ]
        }"#;

        let response = client.post("/offers").body(body).send().await?;

        assert_eq!(response.status(), StatusCode::OK);
        let offer: Offer = response.json()?;
        assert_eq!(offer.items[1].tags, ["metal"]);
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers as they are
#[cfg(test)]
mod shown_tests {
    use serde_json::json;
    use thermite::{TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn an_error_deep_in_the_body_names_its_whole_path() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let body = r#"{
            "name": "Spring sale",
            "items": [
                {"name": "Foo", "price": 35.4},
                {"name": "Bar", "price": 12.0, "image": {"url": "https://example.com/bar.png"}}
            ]
        }"#;

        let response = client.post("/offers").body(body).send().await?;

        assert_eq!(
            response.json::<Value>()?,
            json!({
              "detail": [
                {
                  "type": "missing",
                  "loc": ["body", "items", 1, "image", "name"],
                  "msg": "Field required",
                  "input": {"url": "https://example.com/bar.png"}
                }
              ]
            })
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_list_can_be_the_whole_body() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/images/multiple")
            .body(r#"[{"url": "https://example.com/a.png", "name": "A"}]"#)
            .send()
            .await?;

        assert_eq!(
            response.text()?,
            r#"[{"url":"https://example.com/a.png","name":"A"}]"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_model_inside_itself_is_checked_at_every_depth() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/comments")
            .body(r#"{"text": "a", "replies": [{"text": "b", "replies": [{"text": 3, "replies": []}]}]}"#)
            .send()
            .await?;

        let error: Value = response.json()?;
        assert_eq!(
            error["detail"][0]["loc"],
            json!(["body", "replies", 0, "replies", 0, "text"])
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_list_each_model_once() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        assert_eq!(
            openapi["components"]["schemas"]["Item"],
            json!({
              "properties": {
                "name": {"title": "Name", "type": "string"},
                "price": {"title": "Price", "type": "number"},
                "tags": {
                  "default": [],
                  "items": {"type": "string"},
                  "title": "Tags",
                  "type": "array"
                },
                "image": {
                  "anyOf": [{"$ref": "#/components/schemas/Image"}, {"type": "null"}],
                  "title": "Image"
                }
              },
              "required": ["name", "price"],
              "title": "Item",
              "type": "object"
            })
        );
        client.stop().await;
        Ok(())
    }
}
