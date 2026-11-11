// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{App, Json, Model, Path, Query, RouteError, Server, get, routes};
// ANCHOR_END: imports

// ANCHOR: paging
#[derive(Serialize, Deserialize, Model)]
struct Paging {
    #[model(default = 0)]
    skip: i64,
    #[model(default = 10)]
    limit: i64,
}

#[get("/items")]
async fn read_items(Query(paging): Query<Paging>) -> Json<Paging> {
    Json(paging)
}
// ANCHOR_END: paging

// ANCHOR: optional
#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: String,
}

#[derive(Deserialize, Model)]
struct ItemQuery {
    q: Option<String>,
    #[model(default)]
    short: bool,
}

#[derive(Serialize, Model)]
struct ItemDetails {
    item_id: String,
    q: Option<String>,
    description: Option<String>,
}

#[get("/items/{item_id}")]
async fn read_item(
    Path(ItemPath { item_id }): Path<ItemPath>,
    Query(ItemQuery { q, short }): Query<ItemQuery>,
) -> Json<ItemDetails> {
    let description = if short {
        None
    } else {
        Some("This is an amazing item that has a long description".to_owned())
    };
    Json(ItemDetails {
        item_id,
        q,
        description,
    })
}
// ANCHOR_END: optional

// ANCHOR: required
#[derive(Serialize, Deserialize, Model)]
struct Search {
    needy: String,
    #[model(default)]
    tags: Vec<String>,
}

#[get("/search")]
async fn search(Query(search): Query<Search>) -> Json<Search> {
    Json(search)
}
// ANCHOR_END: required

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![read_items, read_item, search])
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
    async fn fills_what_the_query_leaves_out() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let defaults = client.get("/items").send().await?;
        let given = client.get("/items?skip=20").send().await?;

        assert_eq!(defaults.text()?, r#"{"skip":0,"limit":10}"#);
        assert_eq!(given.text()?, r#"{"skip":20,"limit":10}"#);
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers as they are
#[cfg(test)]
mod shown_tests {
    use thermite::{StatusCode, TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn an_optional_parameter_is_none_when_left_out() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let plain = client.get("/items/foo").send().await?;
        let short = client.get("/items/foo?q=bar&short=true").send().await?;

        assert_eq!(
            plain.text()?,
            concat!(
                r#"{"item_id":"foo","q":null,"#,
                r#""description":"This is an amazing item that has a long description"}"#
            )
        );
        assert_eq!(
            short.text()?,
            r#"{"item_id":"foo","q":"bar","description":null}"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_boolean_is_read_from_the_words_pydantic_reads() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        for word in ["1", "true", "True", "TRUE", "t", "on", "yes", "Yes", "y"] {
            let item: Value = client
                .get(format!("/items/foo?short={word}"))
                .send()
                .await?
                .json()?;
            assert_eq!(item["description"], Value::Null, "{word}");
        }
        for word in ["0", "false", "False", "f", "off", "OFF", "no", "n"] {
            let item: Value = client
                .get(format!("/items/foo?short={word}"))
                .send()
                .await?
                .json()?;
            assert_ne!(item["description"], Value::Null, "{word}");
        }
        let maybe = client.get("/items/foo?short=maybe").send().await?;
        assert_eq!(maybe.status(), StatusCode::UNPROCESSABLE_ENTITY);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_required_parameter_left_out_answers_422() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let missing = client.get("/search").send().await?;
        let tagged = client.get("/search?needy=x&tags=a&tags=b").send().await?;

        assert_eq!(missing.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            missing.text()?,
            concat!(
                r#"{"detail":[{"type":"missing","loc":["query","needy"],"#,
                r#""msg":"Field required","input":null}]}"#
            )
        );
        assert_eq!(tagged.text()?, r#"{"needy":"x","tags":["a","b"]}"#);
        client.stop().await;
        Ok(())
    }
}
