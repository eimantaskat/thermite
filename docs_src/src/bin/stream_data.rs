// ANCHOR: imports
use std::error::Error;
use std::io;
use std::time::Duration;

use futures_util::{StreamExt, stream};
use thermite::{App, RouteError, Server, StreamingResponse, get, routes};
// ANCHOR_END: imports

// ANCHOR: report
#[get("/report")]
async fn report() -> StreamingResponse {
    let rows = stream::iter(1..=3).then(|row| async move {
        // stands in for a query that takes a while for each row
        tokio::time::sleep(Duration::from_millis(10)).await;
        format!("{row},{}\n", row * row)
    });
    StreamingResponse::new(rows).media_type("text/csv")
}
// ANCHOR_END: report

// ANCHOR: export
fn fetch_row(row: i64) -> Result<String, io::Error> {
    if row == 3 {
        return Err(io::Error::other("the database went away"));
    }
    Ok(format!("row {row}\n"))
}

#[get("/export")]
async fn export() -> StreamingResponse {
    StreamingResponse::from_results(stream::iter(1..=5).map(fetch_row))
}
// ANCHOR_END: export

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![report, export])
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
    use thermite::http::header::CONTENT_TYPE;
    use thermite::{Bytes, TestClient};

    use super::*;

    #[tokio::test]
    async fn sends_each_row_as_it_is_made() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let whole = client.get("/report").send().await?;
        let mut streamed = client.get("/report").stream().await?;
        let first = streamed.chunk().await.transpose()?;

        assert_eq!(whole.headers()[CONTENT_TYPE], "text/csv; charset=utf-8");
        assert_eq!(whole.text()?, "1,1\n2,4\n3,9\n");
        assert_eq!(first, Some(Bytes::from("1,1\n")));
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers as they are
#[cfg(test)]
mod shown_tests {
    use thermite::http::header::{CONTENT_LENGTH, TRANSFER_ENCODING};
    use thermite::{Bytes, TestClient, TestRequestError};

    use super::*;

    #[tokio::test]
    async fn sends_the_report_in_chunks() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/report").send().await?;

        assert_eq!(response.headers()[TRANSFER_ENCODING], "chunked");
        assert!(response.headers().get(CONTENT_LENGTH).is_none());
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn an_error_cuts_the_body_short() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let whole = client.get("/export").send().await;
        let mut streamed = client.get("/export").stream().await?;
        let mut chunks = Vec::new();
        while let Some(Ok(chunk)) = streamed.chunk().await {
            chunks.push(chunk);
        }

        assert!(matches!(whole, Err(TestRequestError::ResponseBody(_))));
        assert_eq!(chunks, [Bytes::from("row 1\n"), Bytes::from("row 2\n")]);
        client.stop().await;
        Ok(())
    }
}
