// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{App, Json, Model, Path, Query, RouteError, Server, post, put, routes};
// ANCHOR_END: imports

// ANCHOR: model
#[derive(Serialize, Deserialize, Model)]
struct Item {
    name: String,
    description: Option<String>,
    price: f64,
    tax: Option<f64>,
}
// ANCHOR_END: model

// ANCHOR: create
#[post("/items")]
async fn create_item(Json(item): Json<Item>) -> Json<Item> {
    Json(item)
}
// ANCHOR_END: create

// ANCHOR: update
#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: i64,
}

#[derive(Deserialize, Model)]
struct UpdateQuery {
    q: Option<String>,
}

#[derive(Serialize, Model)]
struct Updated {
    item_id: i64,
    q: Option<String>,
    item: Item,
}

#[put("/items/{item_id}")]
async fn update_item(
    Path(ItemPath { item_id }): Path<ItemPath>,
    Query(UpdateQuery { q }): Query<UpdateQuery>,
    Json(item): Json<Item>,
) -> Json<Updated> {
    Json(Updated { item_id, q, item })
}
// ANCHOR_END: update

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![create_item, update_item])
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
    async fn creates_an_item_from_its_json() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let item = Item {
            name: "Foo".to_owned(),
            description: None,
            price: 45.2,
            tax: None,
        };

        let response = client.post("/items").json(&item).send().await?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.text()?,
            r#"{"name":"Foo","description":null,"price":45.2,"tax":null}"#
        );
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
    async fn reports_every_problem_in_the_body() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/items")
            .body(r#"{"description": "A very nice Item", "price": "cheap"}"#)
            .send()
            .await?;

        assert_eq!(
            response.json::<Value>()?,
            json!({
              "detail": [
                {
                  "type": "missing",
                  "loc": ["body", "name"],
                  "msg": "Field required",
                  "input": {"description": "A very nice Item", "price": "cheap"}
                },
                {
                  "type": "float_parsing",
                  "loc": ["body", "price"],
                  "msg": "Input should be a valid number, unable to parse string as a number",
                  "input": "cheap"
                }
              ]
            })
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_number_sent_as_a_string_is_read() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/items")
            .body(r#"{"name": "Foo", "price": "45.2"}"#)
            .send()
            .await?;

        assert_eq!(response.json::<Value>()?["price"], json!(45.2));
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_body_that_is_not_json_is_reported_where_it_breaks() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/items")
            .body(r#"{"name": "Foo",}"#)
            .send()
            .await?;

        assert_eq!(
            response.json::<Value>()?,
            json!({
              "detail": [
                {
                  "type": "json_invalid",
                  "loc": ["body", 15],
                  "msg": "JSON decode error",
                  "input": {},
                  "ctx": {"error": "trailing comma"}
                }
              ]
            })
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn reads_the_path_the_query_and_the_body() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .put("/items/5?q=x")
            .body(r#"{"name": "Foo", "price": 45.2}"#)
            .send()
            .await?;

        assert_eq!(
            response.text()?,
            concat!(
                r#"{"item_id":5,"q":"x","#,
                r#""item":{"name":"Foo","description":null,"price":45.2,"tax":null}}"#
            )
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_describe_the_model() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        assert_eq!(
            openapi["components"]["schemas"]["Item"],
            json!({
              "properties": {
                "name": {"title": "Name", "type": "string"},
                "description": {
                  "anyOf": [{"type": "string"}, {"type": "null"}],
                  "title": "Description"
                },
                "price": {"title": "Price", "type": "number"},
                "tax": {
                  "anyOf": [{"type": "number"}, {"type": "null"}],
                  "title": "Tax"
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
