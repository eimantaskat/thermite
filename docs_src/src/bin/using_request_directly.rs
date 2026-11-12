// an extractor is written the way users write one, as an async fn that may not await
#![allow(clippy::unused_async_trait_impl)]

// ANCHOR: imports
use std::convert::Infallible;
use std::error::Error;
use std::sync::atomic::{AtomicI64, Ordering};

use serde::{Deserialize, Serialize};
use thermite::http::header::USER_AGENT;
use thermite::{
    App, Arguments, FromRequest, HttpError, Json, Model, Next, Path, Request, RouteError, Server,
    StatusCode, get, routes,
};
// ANCHOR_END: imports

// ANCHOR: extractor
struct RequestInfo {
    method: String,
    path: String,
    user_agent: Option<String>,
}

impl FromRequest for RequestInfo {
    type Rejection = Infallible;

    async fn from_request(request: &Request, _: &Arguments) -> Result<Self, Infallible> {
        Ok(Self {
            method: request.method().to_string(),
            path: request.uri().path().to_owned(),
            user_agent: request
                .headers()
                .get(USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
        })
    }
}
// ANCHOR_END: extractor

// ANCHOR: request_id
#[derive(Clone, Copy)]
struct RequestId(i64);

impl FromRequest for RequestId {
    type Rejection = HttpError;

    async fn from_request(request: &Request, _: &Arguments) -> Result<Self, HttpError> {
        request
            .extensions()
            .get::<RequestId>()
            .copied()
            .ok_or_else(|| {
                HttpError::new(StatusCode::INTERNAL_SERVER_ERROR, "No request id was set")
            })
    }
}
// ANCHOR_END: request_id

// ANCHOR: app
#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: String,
}

#[derive(Serialize, Model)]
struct Seen {
    item_id: String,
    method: String,
    path: String,
    user_agent: Option<String>,
    request_id: i64,
}

#[get("/items/{item_id}")]
async fn read_item(
    Path(ItemPath { item_id }): Path<ItemPath>,
    info: RequestInfo,
    RequestId(request_id): RequestId,
) -> Json<Seen> {
    Json(Seen {
        item_id,
        method: info.method,
        path: info.path,
        user_agent: info.user_agent,
        request_id,
    })
}

fn app() -> Result<App, RouteError> {
    let counter = AtomicI64::new(0);
    App::new()
        .middleware(move |mut request: Request, next: Next| {
            let id = counter.fetch_add(1, Ordering::Relaxed) + 1;
            request.extensions_mut().insert(RequestId(id));
            next.run(request)
        })
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

    use super::*;

    #[tokio::test]
    async fn reads_the_request_and_what_the_middleware_left_in_it() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let first = client.get("/items/foo").send().await?;
        let second = client
            .get("/items/bar")
            .header(USER_AGENT, "curl/8.0")
            .send()
            .await?;

        assert_eq!(
            first.text()?,
            concat!(
                r#"{"item_id":"foo","method":"GET","path":"/items/foo","#,
                r#""user_agent":null,"request_id":1}"#
            )
        );
        assert_eq!(
            second.text()?,
            concat!(
                r#"{"item_id":"bar","method":"GET","path":"/items/bar","#,
                r#""user_agent":"curl/8.0","request_id":2}"#
            )
        );
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows this answer as it is
#[cfg(test)]
mod shown_tests {
    use thermite::TestClient;

    use super::*;

    #[tokio::test]
    async fn answers_500_when_no_middleware_set_the_id() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(App::new().routes(routes![read_item])?).await?;

        let response = client.get("/items/foo").send().await?;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.text()?, r#"{"detail":"No request id was set"}"#);
        client.stop().await;
        Ok(())
    }
}
