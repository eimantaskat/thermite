use std::error::Error;

use serde::Deserialize;
use serde_json::{Value, json};
use thermite::http::header::CONTENT_TYPE;
use thermite::{
    App, Form, Json, Model, RouteError, StatusCode, TestClient, TestMultipart, UploadFile,
};

use crate::support::{RunningServer, TestResult};

#[derive(Deserialize, Model)]
struct Upload {
    title: String,
    file: UploadFile,
    #[model(default)]
    extra: Vec<UploadFile>,
    note: Option<UploadFile>,
}

fn describe(file: &UploadFile) -> Value {
    json!({
        "filename": file.filename(),
        "content_type": file.content_type(),
        "size": file.size(),
        "content": String::from_utf8_lossy(file.content()),
    })
}

fn upload_app() -> Result<App, Box<dyn Error>> {
    Ok(
        App::new().post("/upload", |Form(upload): Form<Upload>| async move {
            Json(json!({
                "title": upload.title,
                "file": describe(&upload.file),
                "extra": upload.extra.iter().map(describe).collect::<Vec<_>>(),
                "note": upload.note.as_ref().map(describe),
            }))
        })?,
    )
}

#[tokio::test]
async fn reads_files_and_text_from_a_multipart_form() -> TestResult {
    let client = TestClient::start(upload_app()?).await?;
    let form = TestMultipart::new()
        .text("title", "report")
        .file("file", "a.txt", "text/plain", "hello")
        .file(
            "extra",
            "b.bin",
            "application/octet-stream",
            vec![0xff, 0x00],
        )
        .file("extra", "c.txt", "text/plain", "c");

    let response = client.post("/upload").multipart(&form).send().await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>()?,
        json!({
            "title": "report",
            "file": {"filename": "a.txt", "content_type": "text/plain", "size": 5, "content": "hello"},
            "extra": [
                {"filename": "b.bin", "content_type": "application/octet-stream", "size": 2, "content": "\u{fffd}\u{0}"},
                {"filename": "c.txt", "content_type": "text/plain", "size": 1, "content": "c"}
            ],
            "note": null
        })
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_a_missing_file_and_text_sent_as_one_with_fastapi_422() -> TestResult {
    let client = TestClient::start(upload_app()?).await?;
    let form = TestMultipart::new()
        .text("title", "report")
        .text("note", "not a file");

    let response = client.post("/upload").multipart(&form).send().await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.json::<Value>()?,
        json!({"detail": [
            {
                "type": "missing",
                "loc": ["body", "file"],
                "msg": "Field required",
                "input": {"title": "report", "note": "not a file"}
            },
            {
                "type": "value_error",
                "loc": ["body", "note"],
                "msg": "Value error, Expected UploadFile, received: <class 'str'>",
                "input": "not a file",
                "ctx": {"error": {}}
            }
        ]})
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_a_malformed_multipart_body_with_starlette_400() -> TestResult {
    let client = TestClient::start(upload_app()?).await?;

    let no_boundary = client
        .post("/upload")
        .header(CONTENT_TYPE, "multipart/form-data")
        .body("--x\r\n")
        .send()
        .await?;
    let garbage = client
        .post("/upload")
        .header(CONTENT_TYPE, "multipart/form-data; boundary=x")
        .body("garbage")
        .send()
        .await?;

    assert_eq!(no_boundary.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        no_boundary.json::<Value>()?,
        json!({"detail": "Missing boundary in multipart."})
    );
    assert_eq!(garbage.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        garbage.json::<Value>()?,
        json!({"detail": "Invalid multipart data."})
    );
    client.stop().await;
    Ok(())
}

#[derive(Deserialize, Model)]
struct Login {
    username: String,
}

#[tokio::test]
async fn a_form_of_text_reads_a_multipart_body_too() -> TestResult {
    let app = App::new().post("/login", |Form(login): Form<Login>| async move {
        Json(json!({"username": login.username}))
    })?;
    let client = TestClient::start(app).await?;
    let form = TestMultipart::new().text("username", "ada");

    let response = client.post("/login").multipart(&form).send().await?;

    assert_eq!(response.json::<Value>()?, json!({"username": "ada"}));
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn documents_a_form_holding_files_as_multipart() -> TestResult {
    let client = TestClient::start(upload_app()?).await?;

    let document: Value = client.get("/openapi.json").send().await?.json()?;

    assert_eq!(
        document["paths"]["/upload"]["post"]["requestBody"]["content"],
        json!({"multipart/form-data": {"schema": {"$ref": "#/components/schemas/Upload"}}})
    );
    assert_eq!(
        document["components"]["schemas"]["Upload"]["properties"]["file"],
        json!({"type": "string", "contentMediaType": "application/octet-stream", "title": "File"})
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn a_json_body_holding_a_file_is_refused_when_registered() {
    let refused = App::new().post("/upload", |Json(_): Json<Upload>| async { StatusCode::OK });

    assert!(matches!(refused, Err(RouteError::FileInJsonBody { .. })));
}

#[tokio::test]
async fn a_browser_upload_reaches_the_handler_over_the_wire() -> TestResult {
    let server = RunningServer::start(upload_app()?).await?;
    let body = "--b0und\r\n\
        Content-Disposition: form-data; name=\"title\"\r\n\r\nété\r\n\
        --b0und\r\n\
        Content-Disposition: form-data; name=\"file\"; filename=\"ž.txt\"\r\n\
        Content-Type: text/plain\r\n\r\nline one\r\nline two\r\n\
        --b0und--\r\n";

    let response = reqwest::Client::new()
        .post(server.url("/upload"))
        .header(CONTENT_TYPE, "multipart/form-data; boundary=b0und")
        .body(body)
        .send()
        .await?;
    let status = response.status();
    let body: Value = serde_json::from_slice(&response.bytes().await?)?;
    server.stop().await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], "été");
    assert_eq!(body["file"]["filename"], "ž.txt");
    assert_eq!(body["file"]["content"], "line one\r\nline two");
    Ok(())
}

#[derive(Deserialize, Model)]
struct Avatar {
    image: UploadFile,
}

#[tokio::test]
async fn an_upload_gives_its_part_headers_and_its_content() -> TestResult {
    let app = App::new().post("/avatar", |Form(avatar): Form<Avatar>| async move {
        let disposition = avatar.image.headers()["content-disposition"]
            .to_str()
            .map(str::to_owned);
        let content = avatar.image.into_content();
        Json(json!({"disposition": disposition.ok(), "content": content.len()}))
    })?;
    let client = TestClient::start(app).await?;
    let form = TestMultipart::new().file("image", "me.png", "image/png", vec![1, 2, 3]);

    let response = client.post("/avatar").multipart(&form).send().await?;

    assert_eq!(
        response.json::<Value>()?,
        json!({
            "disposition": "form-data; name=\"image\"; filename=\"me.png\"",
            "content": 3
        })
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn a_file_input_left_empty_is_missing_like_fastapi() -> TestResult {
    let client = TestClient::start(upload_app()?).await?;
    let form = TestMultipart::new()
        .text("title", "report")
        .file("file", "a.txt", "text/plain", "a")
        .file("extra", "", "application/octet-stream", "")
        .file("note", "", "application/octet-stream", "");
    let urlencoded = client
        .post("/upload")
        .form([("title", "report"), ("file", "a.txt")])
        .send()
        .await?;

    let response = client.post("/upload").multipart(&form).send().await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.json::<Value>()?;
    assert_eq!(body["extra"], json!([]));
    assert_eq!(body["note"], Value::Null);
    assert_eq!(urlencoded.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        urlencoded.json::<Value>()?["detail"][0]["type"],
        "value_error"
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_a_text_field_over_starlette_limit_with_400() -> TestResult {
    let client = TestClient::start(upload_app()?).await?;
    let form = TestMultipart::new().text("title", "a".repeat(1024 * 1024 + 1));

    let response = client.post("/upload").multipart(&form).send().await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.json::<Value>()?,
        json!({"detail": "Part exceeded maximum size of 1024KB."})
    );
    client.stop().await;
    Ok(())
}
