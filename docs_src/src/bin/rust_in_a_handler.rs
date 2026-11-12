// ANCHOR: imports
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use thermite::{
    App, HttpError, Json, Model, Path, RouteError, Server, State, StatusCode, get, post, routes,
};
// ANCHOR_END: imports

// ANCHOR: store
#[derive(Clone, Deserialize, Serialize, Model)]
struct Item {
    name: String,
    price: f64,
}

#[derive(Default)]
struct Store(Mutex<HashMap<String, Item>>);

impl Store {
    fn add(&self, item: Item) -> Result<(), StoreError> {
        let mut items = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if items.contains_key(&item.name) {
            return Err(StoreError::Taken(item.name));
        }
        items.insert(item.name.clone(), item);
        Ok(())
    }

    fn find(&self, name: &str) -> Result<Item, StoreError> {
        let items = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        items
            .get(name)
            .cloned()
            .ok_or_else(|| StoreError::Missing(name.to_owned()))
    }
}
// ANCHOR_END: store

// ANCHOR: error
#[derive(Debug)]
enum StoreError {
    Missing(String),
    Taken(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(name) => write!(formatter, "no item named {name}"),
            Self::Taken(name) => write!(formatter, "an item named {name} already exists"),
        }
    }
}

impl Error for StoreError {}

impl From<StoreError> for HttpError {
    fn from(error: StoreError) -> Self {
        let status = match error {
            StoreError::Missing(_) => StatusCode::NOT_FOUND,
            StoreError::Taken(_) => StatusCode::CONFLICT,
        };
        HttpError::new(status, error.to_string())
    }
}
// ANCHOR_END: error

// ANCHOR: handlers
#[derive(Serialize, Model)]
struct Message {
    message: String,
}

#[post("/items")]
async fn create_item(
    State(store): State<Store>,
    Json(item): Json<Item>,
) -> Result<Json<Message>, HttpError> {
    let message = format!("Added {}", item.name);
    store.add(item)?;
    Ok(Json(Message { message }))
}

#[derive(Deserialize, Model)]
struct ItemPath {
    name: String,
}

#[get("/items/{name}")]
async fn read_item(
    State(store): State<Store>,
    Path(ItemPath { name }): Path<ItemPath>,
) -> Result<Json<Item>, HttpError> {
    let item = store.find(&name)?;
    Ok(Json(item))
}
// ANCHOR_END: handlers

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new()
        .state(Store::default())
        .routes(routes![create_item, read_item])
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
    async fn an_added_item_can_be_read_back() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let item = Item {
            name: "Foo".to_owned(),
            price: 3.5,
        };

        let added = client.post("/items").json(&item).send().await?;
        let read = client.get("/items/Foo").send().await?;

        assert_eq!(added.text()?, r#"{"message":"Added Foo"}"#);
        assert_eq!(read.text()?, r#"{"name":"Foo","price":3.5}"#);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_store_error_answers_its_status() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let item = Item {
            name: "Foo".to_owned(),
            price: 3.5,
        };

        client.post("/items").json(&item).send().await?;
        let again = client.post("/items").json(&item).send().await?;
        let missing = client.get("/items/Bar").send().await?;

        assert_eq!(again.status(), StatusCode::CONFLICT);
        assert_eq!(
            again.text()?,
            r#"{"detail":"an item named Foo already exists"}"#
        );
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.text()?, r#"{"detail":"no item named Bar"}"#);
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test
