// written as async fns, as a dependency that reads a database would be
#![allow(clippy::unused_async_trait_impl)]

// ANCHOR: imports
use std::error::Error;

use thermite::{App, RouteError, Server, get, routes};
// ANCHOR_END: imports

// ANCHOR: dependencies
mod dependencies {
    use serde::Deserialize;
    use thermite::{Dependency, Header, HttpError, Model, StatusCode};

    #[derive(Deserialize, Model)]
    pub struct TokenHeader {
        x_token: String,
    }

    #[derive(Clone)]
    pub struct VerifiedToken;

    impl Dependency for VerifiedToken {
        type Needs = Header<TokenHeader>;
        type Rejection = HttpError;

        async fn resolve(Header(header): Header<TokenHeader>) -> Result<Self, HttpError> {
            if header.x_token == "fake-super-secret-token" {
                Ok(Self)
            } else {
                Err(HttpError::new(
                    StatusCode::BAD_REQUEST,
                    "X-Token header invalid",
                ))
            }
        }
    }
}
// ANCHOR_END: dependencies

// ANCHOR: users
mod users {
    use serde::{Deserialize, Serialize};
    use thermite::{Json, Model, Path, Router, get, routes};

    #[derive(Serialize, Model)]
    struct User {
        username: String,
    }

    #[derive(Deserialize, Model)]
    struct UserPath {
        username: String,
    }

    #[get("/users")]
    async fn read_users() -> Json<Vec<User>> {
        Json(vec![
            User {
                username: "Rick".to_owned(),
            },
            User {
                username: "Morty".to_owned(),
            },
        ])
    }

    #[get("/users/me")]
    async fn read_user_me() -> Json<User> {
        Json(User {
            username: "fakecurrentuser".to_owned(),
        })
    }

    #[get("/users/{username}")]
    async fn read_user(Path(UserPath { username }): Path<UserPath>) -> Json<User> {
        Json(User { username })
    }

    pub fn router() -> Router {
        Router::new()
            .tag("users")
            .routes(routes![read_users, read_user_me, read_user])
    }
}
// ANCHOR_END: users

// ANCHOR: items
mod items {
    use serde::{Deserialize, Serialize};
    use thermite::{Depends, HttpError, Json, Model, Path, Router, StatusCode, get, routes};

    use crate::dependencies::VerifiedToken;

    #[derive(Serialize, Model)]
    struct Item {
        item_id: String,
        name: String,
    }

    #[derive(Deserialize, Model)]
    struct ItemPath {
        item_id: String,
    }

    #[get("")]
    async fn read_items(_: Depends<VerifiedToken>) -> Json<Vec<String>> {
        Json(vec!["plumbus".to_owned(), "gun".to_owned()])
    }

    #[get("/{item_id}")]
    async fn read_item(
        _: Depends<VerifiedToken>,
        Path(ItemPath { item_id }): Path<ItemPath>,
    ) -> Result<Json<Item>, HttpError> {
        let name = match item_id.as_str() {
            "plumbus" => "Plumbus",
            "gun" => "Portal Gun",
            _ => return Err(HttpError::new(StatusCode::NOT_FOUND, "Item not found")),
        };
        Ok(Json(Item {
            item_id,
            name: name.to_owned(),
        }))
    }

    pub fn router() -> Router {
        Router::new()
            .prefix("/items")
            .tag("items")
            .routes(routes![read_items, read_item])
    }
}
// ANCHOR_END: items

// ANCHOR: app
#[get("/")]
async fn root() -> &'static str {
    "Hello Bigger Applications!"
}

fn app() -> Result<App, RouteError> {
    App::new()
        .include_router(users::router())?
        .include_router(items::router())?
        .routes(routes![root])
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
    async fn serves_each_router_under_its_prefix() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let users = client.get("/users").send().await?;
        let item = client
            .get("/items/plumbus")
            .header("x-token", "fake-super-secret-token")
            .send()
            .await?;

        assert_eq!(
            users.text()?,
            r#"[{"username":"Rick"},{"username":"Morty"}]"#
        );
        assert_eq!(item.text()?, r#"{"item_id":"plumbus","name":"Plumbus"}"#);
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
    async fn every_items_route_checks_the_token() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let listed = client
            .get("/items")
            .header("x-token", "fake-super-secret-token")
            .send()
            .await?;
        let wrong = client
            .get("/items/gun")
            .header("x-token", "guess")
            .send()
            .await?;
        let root = client.get("/").send().await?;

        assert_eq!(listed.text()?, r#"["plumbus","gun"]"#);
        assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
        assert_eq!(wrong.text()?, r#"{"detail":"X-Token header invalid"}"#);
        assert_eq!(root.text()?, "Hello Bigger Applications!");
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_group_the_routes_by_tag() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        let operation = &openapi["paths"]["/items/{item_id}"]["get"];
        assert_eq!(operation["tags"], json!(["items"]));
        assert_eq!(operation["operationId"], "read_item_items__item_id__get");
        assert_eq!(
            openapi["paths"]["/users/me"]["get"]["tags"],
            json!(["users"])
        );
        assert!(openapi["paths"]["/"]["get"].get("tags").is_none());
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_fixed_segment_comes_before_a_parameter() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let me = client.get("/users/me").send().await?;
        let rick = client.get("/users/rick").send().await?;
        let missing = client
            .get("/items/nope")
            .header("x-token", "fake-super-secret-token")
            .send()
            .await?;

        assert_eq!(me.text()?, r#"{"username":"fakecurrentuser"}"#);
        assert_eq!(rick.text()?, r#"{"username":"rick"}"#);
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.text()?, r#"{"detail":"Item not found"}"#);
        client.stop().await;
        Ok(())
    }
}
