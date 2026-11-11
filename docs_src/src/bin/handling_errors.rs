// ANCHOR: imports
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use thermite::http::{HeaderName, HeaderValue};
use thermite::{App, HttpError, Json, Model, Path, RouteError, Server, StatusCode, get, routes};
// ANCHOR_END: imports

// ANCHOR: not_found
#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: String,
}

#[derive(Serialize, Model)]
struct Item {
    item: String,
}

fn find_item(item_id: &str) -> Option<&'static str> {
    match item_id {
        "foo" => Some("The Foo Wrestlers"),
        _ => None,
    }
}

#[get("/items/{item_id}")]
async fn read_item(Path(ItemPath { item_id }): Path<ItemPath>) -> Result<Json<Item>, HttpError> {
    match find_item(&item_id) {
        Some(item) => Ok(Json(Item {
            item: item.to_owned(),
        })),
        None => Err(HttpError::new(StatusCode::NOT_FOUND, "Item not found")),
    }
}
// ANCHOR_END: not_found

// ANCHOR: header
#[get("/items-header/{item_id}")]
async fn read_item_header(
    Path(ItemPath { item_id }): Path<ItemPath>,
) -> Result<Json<Item>, HttpError> {
    let item = find_item(&item_id).ok_or_else(|| {
        HttpError::new(StatusCode::NOT_FOUND, "Item not found").with_header(
            HeaderName::from_static("x-error"),
            HeaderValue::from_static("There goes my error"),
        )
    })?;
    Ok(Json(Item {
        item: item.to_owned(),
    }))
}
// ANCHOR_END: header

// ANCHOR: unicorn
#[derive(Debug)]
struct UnicornError {
    name: String,
}

impl fmt::Display for UnicornError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} is a unicorn", self.name)
    }
}

impl Error for UnicornError {}

#[derive(Deserialize, Model)]
struct UnicornPath {
    name: String,
}

#[derive(Serialize, Model)]
struct Message {
    message: String,
}

#[get("/unicorns/{name}")]
async fn read_unicorn(
    Path(UnicornPath { name }): Path<UnicornPath>,
) -> Result<Json<Message>, thermite::Error> {
    if name == "yolo" {
        return Err(UnicornError { name }.into());
    }
    Ok(Json(Message {
        message: format!("{name} is no unicorn"),
    }))
}
// ANCHOR_END: unicorn

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new()
        .exception_handler(|error: &UnicornError| {
            let message = format!(
                "Oops! {} did something. There goes a rainbow...",
                error.name
            );
            (StatusCode::IM_A_TEAPOT, Json(Message { message }))
        })
        .routes(routes![read_item, read_item_header, read_unicorn])
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
    async fn answers_404_for_an_unknown_item() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let found = client.get("/items/foo").send().await?;
        let missing = client.get("/items/bar").send().await?;

        assert_eq!(found.text()?, r#"{"item":"The Foo Wrestlers"}"#);
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.text()?, r#"{"detail":"Item not found"}"#);
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers as they are
#[cfg(test)]
mod shown_tests {
    use thermite::{TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn an_http_error_carries_its_headers() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let missing = client.get("/items-header/bar").send().await?;

        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.headers()["x-error"], "There goes my error");
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_handler_for_an_error_type_answers_for_it() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let yolo = client.get("/unicorns/yolo").send().await?;
        let ada = client.get("/unicorns/ada").send().await?;

        assert_eq!(yolo.status(), StatusCode::IM_A_TEAPOT);
        assert_eq!(
            yolo.text()?,
            r#"{"message":"Oops! yolo did something. There goes a rainbow..."}"#
        );
        assert_eq!(ada.text()?, r#"{"message":"ada is no unicorn"}"#);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_list_only_the_success() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        let responses = &openapi["paths"]["/items/{item_id}"]["get"]["responses"];
        let statuses: Vec<_> = responses
            .as_object()
            .into_iter()
            .flat_map(|responses| responses.keys())
            .collect();
        assert_eq!(statuses, ["200", "422"]);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn an_error_without_a_handler_answers_500() -> Result<(), Box<dyn Error>> {
        let app = App::new().routes(routes![read_unicorn])?;
        let client = TestClient::start(app).await?;

        let response = client.get("/unicorns/yolo").send().await?;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.text()?, "Internal Server Error");
        client.stop().await;
        Ok(())
    }
}
