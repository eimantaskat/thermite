// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{App, Form, Json, Model, RouteError, Server, UploadFile, post, routes};
// ANCHOR_END: imports

// ANCHOR: upload
#[derive(Deserialize, Model)]
struct Upload {
    file: UploadFile,
}

#[derive(Serialize, Model)]
struct Uploaded {
    filename: String,
    content_type: Option<String>,
    size: i64,
}

impl Uploaded {
    fn of(file: &UploadFile) -> Self {
        Self {
            filename: file.filename().to_owned(),
            content_type: file.content_type().map(str::to_owned),
            size: i64::try_from(file.size()).unwrap_or(i64::MAX),
        }
    }
}

#[post("/files")]
async fn upload(Form(upload): Form<Upload>) -> Json<Uploaded> {
    Json(Uploaded::of(&upload.file))
}
// ANCHOR_END: upload

// ANCHOR: many
#[derive(Deserialize, Model)]
struct Report {
    title: String,
    #[model(default)]
    attachments: Vec<UploadFile>,
    cover: Option<UploadFile>,
}

#[derive(Serialize, Model)]
struct Received {
    title: String,
    attachments: Vec<Uploaded>,
    cover: Option<Uploaded>,
}

#[post("/reports")]
async fn create_report(Form(report): Form<Report>) -> Json<Received> {
    Json(Received {
        title: report.title,
        attachments: report.attachments.iter().map(Uploaded::of).collect(),
        cover: report.cover.as_ref().map(Uploaded::of),
    })
}
// ANCHOR_END: many

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![upload, create_report])
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
    use thermite::{TestClient, TestMultipart};

    use super::*;

    #[tokio::test]
    async fn reads_an_uploaded_file() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let form = TestMultipart::new().file("file", "notes.txt", "text/plain", "hello");

        let response = client.post("/files").multipart(&form).send().await?;

        assert_eq!(
            response.text()?,
            r#"{"filename":"notes.txt","content_type":"text/plain","size":5}"#
        );
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers and parts of the schema as they are
#[cfg(test)]
mod shown_tests {
    use serde_json::json;
    use thermite::{TestClient, TestMultipart, Value};

    use super::*;

    #[tokio::test]
    async fn reads_text_and_any_number_of_files() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let form = TestMultipart::new()
            .text("title", "June")
            .file("attachments", "a.csv", "text/csv", "1,2")
            .file("attachments", "b.csv", "text/csv", "3,4,5");
        let bare = TestMultipart::new().text("title", "July");

        let full = client.post("/reports").multipart(&form).send().await?;
        let none = client.post("/reports").multipart(&bare).send().await?;

        assert_eq!(
            full.text()?,
            concat!(
                r#"{"title":"June","attachments":["#,
                r#"{"filename":"a.csv","content_type":"text/csv","size":3},"#,
                r#"{"filename":"b.csv","content_type":"text/csv","size":5}"#,
                r#"],"cover":null}"#
            )
        );
        assert_eq!(
            none.text()?,
            r#"{"title":"July","attachments":[],"cover":null}"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_list_a_multipart_body() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        assert_eq!(
            openapi["paths"]["/files"]["post"]["requestBody"]["content"],
            json!({
              "multipart/form-data": {
                "schema": {"$ref": "#/components/schemas/Upload"}
              }
            })
        );
        assert_eq!(
            openapi["components"]["schemas"]["Upload"]["properties"]["file"],
            json!({
              "type": "string",
              "contentMediaType": "application/octet-stream",
              "title": "File"
            })
        );
        client.stop().await;
        Ok(())
    }
}
