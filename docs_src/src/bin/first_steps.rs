// ANCHOR: all
use std::error::Error;

use serde::Serialize;
use thermite::{App, Json, Model, RouteError, Server, get, routes};

// ANCHOR: model
#[derive(Serialize, Model)]
struct Message {
    message: String,
}
// ANCHOR_END: model

// ANCHOR: handler
#[get("/")]
async fn root() -> Json<Message> {
    Json(Message {
        message: "Hello World".to_owned(),
    })
}
// ANCHOR_END: handler

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![root])
}
// ANCHOR_END: app

// ANCHOR: main
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = Server::bind("127.0.0.1:8000").await?;
    server.serve(app()?).await?;
    Ok(())
}
// ANCHOR_END: main
// ANCHOR_END: all

// ANCHOR: test
#[cfg(test)]
mod tests {
    use thermite::{StatusCode, TestClient};

    use super::*;

    #[tokio::test]
    async fn root_answers_hello_world() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/").send().await?;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.text()?, r#"{"message":"Hello World"}"#);
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows this part of the schema as it is
#[cfg(test)]
mod openapi_tests {
    use serde_json::json;
    use thermite::{StatusCode, TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn the_docs_describe_root() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let docs = client.get("/docs").send().await?;
        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        assert_eq!(docs.status(), StatusCode::OK);
        assert!(docs.text()?.contains(r#"url: "/openapi.json""#));
        assert_eq!(
            openapi["paths"],
            json!({
              "/": {
                "get": {
                  "summary": "Root",
                  "operationId": "root__get",
                  "responses": {
                    "200": {
                      "description": "Successful Response",
                      "content": {
                        "application/json": {
                          "schema": {"$ref": "#/components/schemas/Message"}
                        }
                      }
                    }
                  }
                }
              }
            })
        );
        client.stop().await;
        Ok(())
    }
}
