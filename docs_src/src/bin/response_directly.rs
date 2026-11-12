// ANCHOR: imports
use std::error::Error;

use serde::Serialize;
use thermite::http::HeaderValue;
use thermite::http::header::{CONTENT_TYPE, LOCATION, SET_COOKIE};
use thermite::{
    App, Body, Handler, IntoResponse, Json, Model, Response, RouteError, Server, StatusCode, get,
    routes,
};
// ANCHOR_END: imports

// ANCHOR: legacy
const LEGACY_DATA: &str = r#"<?xml version="1.0"?>
<shampoo>
<Header>Apply shampoo here.</Header>
<Body>You'll have to use soap here.</Body>
</shampoo>
"#;

#[get("/legacy")]
async fn get_legacy_data() -> Response {
    let mut response = Response::new(Body::from(LEGACY_DATA));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));
    response
}
// ANCHOR_END: legacy

// ANCHOR: headers
#[derive(Serialize, Model)]
struct Message {
    message: String,
}

async fn get_headers() -> Response {
    let message = Message {
        message: "Hello World".to_owned(),
    };
    let mut response = Json(message).into_response();
    response
        .headers_mut()
        .insert("x-cat-dog", HeaderValue::from_static("alone in the world"));
    response
}
// ANCHOR_END: headers

// ANCHOR: cookies
#[get("/cookie")]
async fn create_cookie() -> Response {
    let mut response = "Come to the dark side, we have cookies".into_response();
    let headers = response.headers_mut();
    headers.append(
        SET_COOKIE,
        HeaderValue::from_static("fakesession=fake-cookie-session-value; Path=/; SameSite=lax"),
    );
    headers.append(
        SET_COOKIE,
        HeaderValue::from_static("theme=dark; Path=/; SameSite=lax"),
    );
    response
}
// ANCHOR_END: cookies

// ANCHOR: redirect
#[get("/typer")]
async fn redirect_typer() -> (StatusCode, Response) {
    let mut response = Response::new(Body::empty());
    response.headers_mut().insert(
        LOCATION,
        HeaderValue::from_static("https://typer.tiangolo.com"),
    );
    (StatusCode::TEMPORARY_REDIRECT, response)
}
// ANCHOR_END: redirect

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new()
        .routes(routes![get_legacy_data, create_cookie, redirect_typer])?
        .get(
            "/headers",
            get_headers.response_with_body::<Message>(StatusCode::OK, "Successful Response"),
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
    async fn sends_the_response_as_it_was_built() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let legacy = client.get("/legacy").send().await?;
        let headers = client.get("/headers").send().await?;

        assert_eq!(legacy.headers()[CONTENT_TYPE], "application/xml");
        assert_eq!(legacy.text()?, LEGACY_DATA);
        assert_eq!(headers.headers()["x-cat-dog"], "alone in the world");
        assert_eq!(headers.text()?, r#"{"message":"Hello World"}"#);
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
    async fn sets_each_cookie_in_its_own_header() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/cookie").send().await?;

        let cookies: Vec<_> = response.headers().get_all(SET_COOKIE).iter().collect();
        assert_eq!(
            cookies,
            [
                "fakesession=fake-cookie-session-value; Path=/; SameSite=lax",
                "theme=dark; Path=/; SameSite=lax"
            ]
        );
        assert_eq!(response.text()?, "Come to the dark side, we have cookies");
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn redirects_with_the_status_of_the_pair() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/typer").send().await?;

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers()[LOCATION], "https://typer.tiangolo.com");
        assert!(response.body().is_empty());
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn documents_only_what_the_types_or_the_route_say() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        assert_eq!(
            openapi["paths"]["/legacy"]["get"]["responses"],
            json!({
                "200": {
                    "description": "Successful Response",
                    "content": {"application/json": {"schema": {}}}
                }
            })
        );
        assert_eq!(
            openapi["paths"]["/headers"]["get"]["responses"]["200"]["content"],
            json!({"application/json": {"schema": {"$ref": "#/components/schemas/Message"}}})
        );
        client.stop().await;
        Ok(())
    }
}
