use serde::ser::Error as _;
use serde::{Serialize, Serializer};
use thermite::http::header::CONTENT_TYPE;
use thermite::{App, Json, StatusCode};

use crate::support::{RunningServer, TestResult};

#[derive(Serialize)]
struct Created {
    id: i64,
}

struct Unserializable;

impl Serialize for Unserializable {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(S::Error::custom("not today"))
    }
}

fn find(id: i64) -> Result<String, StatusCode> {
    if id == 1 {
        Ok("found".to_owned())
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

#[tokio::test]
async fn turns_handler_return_values_into_responses() -> TestResult {
    let app = App::new()
        .get("/text", || async { "hot iron" })?
        .delete("/empty", || async { StatusCode::NO_CONTENT })?
        .post("/created", || async {
            (StatusCode::CREATED, Json(Created { id: 7 }))
        })?
        .get("/found", || async { find(1) })?
        .get("/missing", || async { find(2) })?;
    let server = RunningServer::start(app).await?;
    let client = reqwest::Client::new();

    let text = client.get(server.url("/text")).send().await?;
    let empty = client.delete(server.url("/empty")).send().await?;
    let created = client.post(server.url("/created")).send().await?;
    let found = client.get(server.url("/found")).send().await?;
    let missing = client.get(server.url("/missing")).send().await?;

    assert_eq!(text.headers()[CONTENT_TYPE], "text/plain; charset=utf-8");
    assert_eq!(text.text().await?, "hot iron");
    assert_eq!(empty.status(), StatusCode::NO_CONTENT);
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_eq!(created.headers()[CONTENT_TYPE], "application/json");
    assert_eq!(created.text().await?, r#"{"id":7}"#);
    assert_eq!(found.text().await?, "found");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    server.stop().await
}

#[tokio::test]
async fn answers_500_when_a_json_response_does_not_serialize() -> TestResult {
    let app = App::new()
        .get("/broken", || async { Json(Unserializable) })?
        .post("/broken", || async {
            (StatusCode::CREATED, Json(Unserializable))
        })?;
    let server = RunningServer::start(app).await?;

    let plain = reqwest::get(server.url("/broken")).await?;
    let created = reqwest::Client::new()
        .post(server.url("/broken"))
        .send()
        .await?;

    for response in [plain, created] {
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.text().await?, "Internal Server Error");
    }
    server.stop().await
}
