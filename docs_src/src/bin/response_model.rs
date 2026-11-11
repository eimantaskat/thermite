// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{
    App, Handler, HttpError, Json, Model, RouteError, Server, StatusCode, post, routes,
};
// ANCHOR_END: imports

// ANCHOR: users
#[derive(Deserialize, Model)]
struct UserIn {
    username: String,
    password: String,
    email: String,
    full_name: Option<String>,
}

#[derive(Serialize, Model)]
struct UserOut {
    username: String,
    email: String,
    full_name: Option<String>,
}

#[post("/users")]
async fn create_user(Json(user): Json<UserIn>) -> Result<Json<UserOut>, HttpError> {
    if user.password.chars().count() < 8 {
        return Err(HttpError::new(
            StatusCode::BAD_REQUEST,
            "The password needs at least 8 characters",
        ));
    }
    Ok(Json(UserOut {
        username: user.username,
        email: user.email,
        full_name: user.full_name,
    }))
}
// ANCHOR_END: users

// ANCHOR: status
#[derive(Serialize, Deserialize, Model)]
struct Item {
    name: String,
    price: f64,
}

async fn create_item(Json(item): Json<Item>) -> (StatusCode, Json<Item>) {
    (StatusCode::CREATED, Json(item))
}
// ANCHOR_END: status

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![create_user])?.post(
        "/items",
        create_item.response_with_body::<Item>(StatusCode::CREATED, "Successful Response"),
    )
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
    async fn never_answers_with_the_password() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/users")
            .body(r#"{"username": "ada", "password": "correct horse", "email": "ada@example.com"}"#)
            .send()
            .await?;

        assert_eq!(
            response.text()?,
            r#"{"username":"ada","email":"ada@example.com","full_name":null}"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn answers_201_for_a_new_item() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/items")
            .body(r#"{"name": "Foo", "price": 45.2}"#)
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::CREATED);
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers and parts of the schema as they are
#[cfg(test)]
mod shown_tests {
    use serde_json::json;
    use thermite::{TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn refuses_a_short_password() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/users")
            .body(r#"{"username": "ada", "password": "short", "email": "ada@example.com"}"#)
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.text()?,
            r#"{"detail":"The password needs at least 8 characters"}"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_show_what_each_route_answers() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        let paths = &openapi["paths"];
        assert_eq!(
            paths["/users"]["post"]["responses"]["200"],
            json!({
              "description": "Successful Response",
              "content": {
                "application/json": {
                  "schema": {"$ref": "#/components/schemas/UserOut"}
                }
              }
            })
        );
        let items = &paths["/items"]["post"]["responses"];
        assert_eq!(
            items["201"],
            json!({
              "description": "Successful Response",
              "content": {
                "application/json": {
                  "schema": {"$ref": "#/components/schemas/Item"}
                }
              }
            })
        );
        let statuses: Vec<_> = items
            .as_object()
            .into_iter()
            .flat_map(|responses| responses.keys())
            .collect();
        assert_eq!(statuses, ["201", "413", "422"]);
        client.stop().await;
        Ok(())
    }
}
