// written as async fns, as a dependency that reads a database would be
#![allow(clippy::unused_async_trait_impl)]

// ANCHOR: imports
use std::convert::Infallible;
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{
    App, Cookie, Dependency, Depends, Header, HttpError, Json, Model, Query, RouteError, Server,
    StatusCode, get, routes,
};
// ANCHOR_END: imports

// ANCHOR: pagination
#[derive(Deserialize, Model)]
struct PageQuery {
    #[model(default = 1)]
    page: i64,
    #[model(default = 10)]
    size: i64,
}

#[derive(Clone, Serialize, Model)]
struct Pagination {
    skip: i64,
    limit: i64,
}

impl Dependency for Pagination {
    type Needs = Query<PageQuery>;
    type Rejection = HttpError;

    async fn resolve(Query(query): Query<PageQuery>) -> Result<Self, HttpError> {
        if query.page < 1 || query.size < 1 {
            return Err(HttpError::new(
                StatusCode::BAD_REQUEST,
                "page and size start at 1",
            ));
        }
        Ok(Self {
            skip: (query.page - 1) * query.size,
            limit: query.size,
        })
    }
}

#[get("/items")]
async fn read_items(Depends(pagination): Depends<Pagination>) -> Json<Pagination> {
    Json(pagination)
}

#[get("/users")]
async fn read_users(Depends(pagination): Depends<Pagination>) -> Json<Pagination> {
    Json(pagination)
}
// ANCHOR_END: pagination

// ANCHOR: sub_dependencies
#[derive(Deserialize, Model)]
struct SearchQuery {
    q: Option<String>,
}

#[derive(Clone)]
struct SearchTerm(Option<String>);

impl Dependency for SearchTerm {
    type Needs = Query<SearchQuery>;
    type Rejection = Infallible;

    async fn resolve(Query(query): Query<SearchQuery>) -> Result<Self, Infallible> {
        Ok(Self(query.q))
    }
}

#[derive(Deserialize, Model)]
struct LastSearch {
    last_query: Option<String>,
}

#[derive(Clone)]
struct TermOrCookie(Option<String>);

impl Dependency for TermOrCookie {
    type Needs = (Depends<SearchTerm>, Cookie<LastSearch>);
    type Rejection = Infallible;

    async fn resolve(
        (Depends(SearchTerm(term)), Cookie(cookie)): Self::Needs,
    ) -> Result<Self, Infallible> {
        Ok(Self(term.or(cookie.last_query)))
    }
}

#[derive(Serialize, Model)]
struct Searched {
    q_or_cookie: Option<String>,
}

#[get("/search")]
async fn search(Depends(TermOrCookie(term)): Depends<TermOrCookie>) -> Json<Searched> {
    Json(Searched { q_or_cookie: term })
}
// ANCHOR_END: sub_dependencies

// ANCHOR: guard
#[derive(Deserialize, Model)]
struct KeyHeader {
    x_key: String,
}

#[derive(Clone)]
struct VerifiedKey;

impl Dependency for VerifiedKey {
    type Needs = Header<KeyHeader>;
    type Rejection = HttpError;

    async fn resolve(Header(header): Header<KeyHeader>) -> Result<Self, HttpError> {
        if header.x_key == "fake-super-secret-key" {
            Ok(Self)
        } else {
            Err(HttpError::new(
                StatusCode::BAD_REQUEST,
                "X-Key header invalid",
            ))
        }
    }
}

#[get("/reports")]
async fn read_reports(_: Depends<VerifiedKey>) -> &'static str {
    "the reports"
}
// ANCHOR_END: guard

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![read_items, read_users, search, read_reports])
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
    async fn every_route_gets_the_same_pagination() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let items = client.get("/items?page=3&size=5").send().await?;
        let users = client.get("/users").send().await?;

        assert_eq!(items.text()?, r#"{"skip":10,"limit":5}"#);
        assert_eq!(users.text()?, r#"{"skip":0,"limit":10}"#);
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers and parts of the schema as they are
#[cfg(test)]
mod shown_tests {
    use serde_json::json;
    use thermite::http::header::COOKIE;
    use thermite::{TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn a_rejected_dependency_answers_for_the_handler() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/items?page=0").send().await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.text()?, r#"{"detail":"page and size start at 1"}"#);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_list_what_a_dependency_reads() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        assert_eq!(
            openapi["paths"]["/items"]["get"]["parameters"],
            json!([
              {
                "name": "page",
                "in": "query",
                "required": false,
                "schema": {"type": "integer", "default": 1, "title": "Page"}
              },
              {
                "name": "size",
                "in": "query",
                "required": false,
                "schema": {"type": "integer", "default": 10, "title": "Size"}
              }
            ])
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_query_comes_first_and_then_the_cookie() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let both = client
            .get("/search?q=iron")
            .header(COOKIE, "last_query=copper")
            .send()
            .await?;
        let cookie = client
            .get("/search")
            .header(COOKIE, "last_query=copper")
            .send()
            .await?;
        let neither = client.get("/search").send().await?;

        assert_eq!(both.text()?, r#"{"q_or_cookie":"iron"}"#);
        assert_eq!(cookie.text()?, r#"{"q_or_cookie":"copper"}"#);
        assert_eq!(neither.text()?, r#"{"q_or_cookie":null}"#);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_dependency_can_guard_a_route() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let allowed = client
            .get("/reports")
            .header("x-key", "fake-super-secret-key")
            .send()
            .await?;
        let wrong = client
            .get("/reports")
            .header("x-key", "guess")
            .send()
            .await?;
        let missing = client.get("/reports").send().await?;

        assert_eq!(allowed.text()?, "the reports");
        assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
        assert_eq!(wrong.text()?, r#"{"detail":"X-Key header invalid"}"#);
        assert_eq!(missing.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            missing.text()?,
            concat!(
                r#"{"detail":[{"type":"missing","loc":["header","x-key"],"#,
                r#""msg":"Field required","input":null}]}"#
            )
        );
        client.stop().await;
        Ok(())
    }
}
