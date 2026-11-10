use std::error::Error;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thermite::http::header::CONTENT_TYPE;
use thermite::{App, Form, Json, Model, StatusCode, TestClient};

use crate::support::{RunningServer, TestResult};

#[derive(Serialize, Deserialize, Model)]
struct Login {
    username: String,
    password: String,
    #[model(default = false)]
    remember: bool,
    #[model(default = Vec::<i64>::new())]
    scopes: Vec<i64>,
}

fn login_app() -> Result<App, Box<dyn Error>> {
    Ok(App::new().post(
        "/login",
        |Form(login): Form<Login>| async move { Json(login) },
    )?)
}

#[tokio::test]
async fn reads_a_model_from_a_form_body() -> TestResult {
    let client = TestClient::start(login_app()?).await?;

    let response = client
        .post("/login")
        .form([
            ("username", "ada lovelace"),
            ("password", "p&ss=word"),
            ("remember", "on"),
            ("scopes", "1"),
            ("scopes", "2"),
        ])
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>()?,
        json!({
            "username": "ada lovelace",
            "password": "p&ss=word",
            "remember": true,
            "scopes": [1, 2]
        })
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_missing_and_invalid_form_fields_with_fastapi_422() -> TestResult {
    let client = TestClient::start(login_app()?).await?;

    let response = client
        .post("/login")
        .form([("username", "ada"), ("password", ""), ("scopes", "x")])
        .send()
        .await?;
    let json_body = client
        .post("/login")
        .json(&json!({"username": "ada", "password": "secret"}))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.json::<Value>()?,
        json!({"detail": [
            {
                "type": "missing",
                "loc": ["body", "password"],
                "msg": "Field required",
                "input": {"username": "ada", "scopes": ["x"]}
            },
            {
                "type": "int_parsing",
                "loc": ["body", "scopes", 0],
                "msg": "Input should be a valid integer, unable to parse string as an integer",
                "input": "x"
            }
        ]})
    );
    assert_eq!(json_body.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        json_body.json::<Value>()?["detail"][0]["loc"],
        json!(["body", "username"])
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn a_browser_form_post_reaches_the_handler_over_the_wire() -> TestResult {
    let server = RunningServer::start(login_app()?).await?;

    let response = reqwest::Client::new()
        .post(server.url("/login"))
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body("username=ada&password=%C3%A9t%C3%A9")
        .send()
        .await?;
    let status = response.status();
    let body: Value = serde_json::from_slice(&response.bytes().await?)?;
    server.stop().await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["password"], "été");
    Ok(())
}

#[tokio::test]
async fn documents_a_form_body_under_its_media_type() -> TestResult {
    let client = TestClient::start(login_app()?).await?;

    let document: Value = client.get("/openapi.json").send().await?.json()?;

    assert_eq!(
        document["paths"]["/login"]["post"]["requestBody"],
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

#[tokio::test]
async fn the_test_client_encodes_any_text_so_it_reads_back_the_same() -> TestResult {
    let client = TestClient::start(login_app()?).await?;
    let tricky = "a&b=c + 100% *-._~ été";

    let response = client
        .post("/login")
        .form(vec![
            ("username".to_owned(), tricky.to_owned()),
            ("password".to_owned(), "=".to_owned()),
        ])
        .send()
        .await?;

    assert_eq!(response.json::<Value>()?["username"], tricky);
    assert_eq!(response.json::<Value>()?["password"], "=");
    client.stop().await;
    Ok(())
}
