// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{App, Cookie, Header, Json, Model, RouteError, Server, get, routes};
// ANCHOR_END: imports

// ANCHOR: models
#[derive(Deserialize, Model)]
struct Client {
    user_agent: Option<String>,
    #[model(default)]
    x_token: Vec<String>,
}

#[derive(Deserialize, Model)]
struct Session {
    ads_id: Option<String>,
}
// ANCHOR_END: models

// ANCHOR: handler
#[derive(Serialize, Model)]
struct Seen {
    user_agent: Option<String>,
    x_token: Vec<String>,
    ads_id: Option<String>,
}

#[get("/items")]
async fn read_items(
    Header(client): Header<Client>,
    Cookie(session): Cookie<Session>,
) -> Json<Seen> {
    Json(Seen {
        user_agent: client.user_agent,
        x_token: client.x_token,
        ads_id: session.ads_id,
    })
}
// ANCHOR_END: handler

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![read_items])
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
    async fn reads_headers_and_cookies() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .get("/items")
            .header("user-agent", "curl/8.5.0")
            .header("x-token", "foo")
            .header("x-token", "bar")
            .header("cookie", "theme=dark; ads_id=abc")
            .send()
            .await?;

        assert_eq!(
            response.text()?,
            r#"{"user_agent":"curl/8.5.0","x_token":["foo","bar"],"ads_id":"abc"}"#
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
    async fn leaves_out_what_the_request_does_not_send() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/items").send().await?;

        assert_eq!(
            response.text()?,
            r#"{"user_agent":null,"x_token":[],"ads_id":null}"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_list_headers_by_their_names_in_the_request() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        let names: Vec<_> = openapi["paths"]["/items"]["get"]["parameters"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|parameter| (&parameter["name"], &parameter["in"]))
            .collect();
        assert_eq!(
            names,
            [
                (&json!("user-agent"), &json!("header")),
                (&json!("x-token"), &json!("header")),
                (&json!("ads_id"), &json!("cookie")),
            ]
        );
        client.stop().await;
        Ok(())
    }
}
