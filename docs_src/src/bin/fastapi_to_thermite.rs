// ANCHOR: all
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{
    App, HttpError, Json, Model, Path, Query, RouteError, Server, StatusCode, get, post, routes,
};

#[derive(Deserialize, Serialize, Model)]
struct Item {
    name: String,
    price: f64,
    #[model(default)]
    tags: Vec<String>,
}

#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: i64,
}

#[derive(Deserialize, Model)]
struct ItemQuery {
    q: Option<String>,
}

#[derive(Serialize, Model)]
struct ItemRead {
    item_id: i64,
    q: Option<String>,
}

#[get("/items/{item_id}")]
async fn read_item(
    Path(ItemPath { item_id }): Path<ItemPath>,
    Query(ItemQuery { q }): Query<ItemQuery>,
) -> Result<Json<ItemRead>, HttpError> {
    if item_id == 0 {
        return Err(HttpError::new(StatusCode::NOT_FOUND, "Item not found"));
    }
    Ok(Json(ItemRead { item_id, q }))
}

#[post("/items")]
async fn create_item(Json(item): Json<Item>) -> (StatusCode, Json<Item>) {
    (StatusCode::CREATED, Json(item))
}

fn app() -> Result<App, RouteError> {
    App::new().routes(routes![read_item, create_item])
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = Server::bind("127.0.0.1:8000").await?;
    server.serve(app()?).await?;
    Ok(())
}
// ANCHOR_END: all

// ANCHOR: test
#[cfg(test)]
mod tests {
    use thermite::TestClient;

    use super::*;

    #[tokio::test]
    async fn answers_as_the_fastapi_app_does() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let item = Item {
            name: "Foo".to_owned(),
            price: 3.5,
            tags: Vec::new(),
        };

        let read = client.get("/items/5?q=somequery").send().await?;
        let missing = client.get("/items/0").send().await?;
        let created = client.post("/items").json(&item).send().await?;

        assert_eq!(read.text()?, r#"{"item_id":5,"q":"somequery"}"#);
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.text()?, r#"{"detail":"Item not found"}"#);
        assert_eq!(created.status(), StatusCode::CREATED);
        assert_eq!(created.text()?, r#"{"name":"Foo","price":3.5,"tags":[]}"#);
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers as they are
#[cfg(test)]
mod shown_tests {
    use thermite::TestClient;

    use super::*;

    #[tokio::test]
    async fn a_missing_query_is_null() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/items/5").send().await?;

        assert_eq!(response.text()?, r#"{"item_id":5,"q":null}"#);
        client.stop().await;
        Ok(())
    }
}
