// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{App, Form, HttpError, Json, Model, RouteError, Server, StatusCode, post, routes};
// ANCHOR_END: imports

// ANCHOR: login
#[derive(Deserialize, Model)]
struct Login {
    username: String,
    password: String,
    #[model(default)]
    remember: bool,
}

#[derive(Serialize, Model)]
struct Welcome {
    username: String,
    remember: bool,
}

#[post("/login")]
async fn login(Form(login): Form<Login>) -> Result<Json<Welcome>, HttpError> {
    if login.password != "secret" {
        return Err(HttpError::new(
            StatusCode::BAD_REQUEST,
            "Incorrect password",
        ));
    }
    Ok(Json(Welcome {
        username: login.username,
        remember: login.remember,
    }))
}
// ANCHOR_END: login

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![login])
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
    async fn reads_the_fields_of_a_form() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/login")
            .form([
                ("username", "ada"),
                ("password", "secret"),
                ("remember", "on"),
            ])
            .send()
            .await?;

        assert_eq!(response.text()?, r#"{"username":"ada","remember":true}"#);
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
    async fn a_missing_field_answers_422() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/login")
            .form([("username", "ada")])
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response.text()?,
            concat!(
                r#"{"detail":[{"type":"missing","loc":["body","password"],"#,
                r#""msg":"Field required","input":{"username":"ada"}}]}"#
            )
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_list_the_form_body() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        assert_eq!(
            openapi["paths"]["/login"]["post"]["requestBody"],
            json!({
              "content": {
                "application/x-www-form-urlencoded": {
                  "schema": {"$ref": "#/components/schemas/Login"}
                }
              },
              "required": true
            })
        );
        client.stop().await;
        Ok(())
    }

    #[test]
    fn a_route_reads_only_one_body() {
        let both = App::new()
            .post(
                "/both",
                |Form(_): Form<Login>, Json(_): Json<Login>| async {},
            )
            .err()
            .map(|error| error.to_string());

        assert_eq!(both.as_deref(), Some("`/both` declares more than one body"));
    }

    #[tokio::test]
    async fn a_wrong_password_answers_400() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/login")
            .form([("username", "ada"), ("password", "guess")])
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.text()?, r#"{"detail":"Incorrect password"}"#);
        client.stop().await;
        Ok(())
    }
}
