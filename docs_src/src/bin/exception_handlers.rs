// ANCHOR: imports
use std::error::Error;

use serde::Deserialize;
use thermite::{
    App, HttpError, IntoResponse, Model, Path, Response, RouteError, Server, StatusCode, get,
    routes,
};
// ANCHOR_END: imports

// ANCHOR: handler
fn plain_text_error(error: &HttpError) -> Response {
    let detail = match error.detail().as_str() {
        Some(text) => text.to_owned(),
        None => error.detail().to_string(),
    };
    let mut response = (error.status(), detail).into_response();
    response.headers_mut().extend(error.headers().clone());
    response
}
// ANCHOR_END: handler

// ANCHOR: app
#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: i64,
}

#[get("/items/{item_id}")]
async fn read_item(Path(ItemPath { item_id }): Path<ItemPath>) -> Result<String, HttpError> {
    if item_id == 3 {
        return Err(HttpError::new(
            StatusCode::IM_A_TEAPOT,
            "Nope! I don't like 3.",
        ));
    }
    Ok(format!("item {item_id}"))
}

fn app() -> Result<App, RouteError> {
    App::new()
        .exception_handler(plain_text_error)
        .routes(routes![read_item])
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
    use thermite::http::header::{ALLOW, CONTENT_TYPE};

    use super::*;

    #[tokio::test]
    async fn answers_every_http_error_in_plain_text() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let teapot = client.get("/items/3").send().await?;
        let missing = client.get("/users").send().await?;
        let wrong_method = client.delete("/items/1").send().await?;

        assert_eq!(teapot.status(), StatusCode::IM_A_TEAPOT);
        assert_eq!(teapot.headers()[CONTENT_TYPE], "text/plain; charset=utf-8");
        assert_eq!(teapot.text()?, "Nope! I don't like 3.");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.headers()[CONTENT_TYPE], "text/plain; charset=utf-8");
        assert_eq!(missing.text()?, "Not Found");
        assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(wrong_method.headers()[ALLOW], "GET, HEAD");
        assert_eq!(wrong_method.text()?, "Method Not Allowed");
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
    async fn the_handler_leaves_other_answers_alone() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let found = client.get("/items/1").send().await?;
        let invalid = client.get("/items/foo").send().await?;

        assert_eq!(found.text()?, "item 1");
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            invalid
                .text()?
                .starts_with(r#"{"detail":[{"type":"int_parsing","#)
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_keep_the_default_responses() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        let responses = &openapi["paths"]["/items/{item_id}"]["get"]["responses"];
        assert_eq!(
            responses["422"]["content"]["application/json"]["schema"],
            json!({"$ref": "#/components/schemas/HTTPValidationError"})
        );
        assert!(responses.get("418").is_none());
        client.stop().await;
        Ok(())
    }

    #[test]
    fn the_default_answer_is_the_json_detail() {
        let error = HttpError::new(StatusCode::IM_A_TEAPOT, "Nope! I don't like 3.");

        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
        assert_eq!(
            response.body().as_bytes().map(AsRef::as_ref),
            Some(&br#"{"detail":"Nope! I don't like 3."}"#[..])
        );
    }
}
