// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{App, Json, Model, Path, RouteError, Server, get, routes};
// ANCHOR_END: imports

// ANCHOR: item
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
// ANCHOR_END: item

// ANCHOR: users
#[derive(Deserialize, Model)]
struct UserPath {
    user_id: String,
}

#[derive(Serialize, Model)]
struct User {
    user_id: String,
}

#[get("/users/{user_id}")]
async fn read_user(Path(UserPath { user_id }): Path<UserPath>) -> Json<User> {
    Json(User { user_id })
}

#[get("/users/me")]
async fn read_current_user() -> Json<User> {
    Json(User {
        user_id: "the current user".to_owned(),
    })
}
// ANCHOR_END: users

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![read_item, read_user, read_current_user])
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
    async fn reads_the_item_id_as_an_integer() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let found = client.get("/items/3").send().await?;
        let invalid = client.get("/items/foo").send().await?;

        assert_eq!(found.text()?, r#"{"item_id":3}"#);
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
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
    async fn an_invalid_item_id_is_reported_where_it_was() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let foo = client.get("/items/foo").send().await?;
        let decimal: Value = client.get("/items/4.2").send().await?.json()?;

        assert_eq!(
            foo.text()?,
            concat!(
                r#"{"detail":[{"type":"int_parsing","loc":["path","item_id"],"#,
                r#""msg":"Input should be a valid integer, unable to parse string as an integer","#,
                r#""input":"foo"}]}"#
            )
        );
        assert_eq!(decimal["detail"][0]["input"], "4.2");
        assert_eq!(decimal["detail"][0]["type"], "int_parsing");
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_fixed_path_wins_over_a_parameter_in_any_order() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let me = client.get("/users/me").send().await?;
        let other = client.get("/users/ada").send().await?;

        assert_eq!(me.text()?, r#"{"user_id":"the current user"}"#);
        assert_eq!(other.text()?, r#"{"user_id":"ada"}"#);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_list_the_path_parameter() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        assert_eq!(
            openapi["paths"]["/items/{item_id}"]["get"]["parameters"],
            json!([
              {
                "name": "item_id",
                "in": "path",
                "required": true,
                "schema": {"type": "integer", "title": "Item Id"}
              }
            ])
        );
        client.stop().await;
        Ok(())
    }

    #[test]
    fn a_field_without_a_segment_is_refused() {
        #[derive(Deserialize, Model)]
        struct FilePath {
            id: i64,
        }

        #[get("/files/{file_id}")]
        async fn read_file(Path(FilePath { id }): Path<FilePath>) -> String {
            id.to_string()
        }

        let error = App::new().routes(routes![read_file]).err();

        assert_eq!(
            error.map(|error| error.to_string()),
            Some(
                "path `/files/{file_id}` has no segment `{id}` for its path parameter `id`"
                    .to_owned()
            )
        );
    }
}
