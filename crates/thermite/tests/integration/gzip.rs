use std::error::Error;
use std::io::Read;

use flate2::read::GzDecoder;
use futures_util::{StreamExt, stream};
use thermite::http::HeaderValue;
use thermite::http::header::{
    ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, TRANSFER_ENCODING, VARY,
};
use thermite::{App, GZipMiddleware, Response, StatusCode, StreamingResponse, TestClient};

use crate::support::{RunningServer, TestResult};

const LONG: usize = 4000;

fn long_text() -> String {
    "x".repeat(LONG)
}

fn decompress(compressed: &[u8]) -> Result<String, std::io::Error> {
    let mut text = String::new();
    GzDecoder::new(compressed).read_to_string(&mut text)?;
    Ok(text)
}

fn app(gzip: GZipMiddleware) -> Result<App, Box<dyn Error>> {
    Ok(App::new()
        .middleware(gzip)
        .get("/long", || async { long_text() })?
        .get("/short", || async { "OK" })?
        .get("/streamed", || async {
            StreamingResponse::new(stream::repeat("x".repeat(400)).take(10))
                .media_type("text/plain")
        })?
        .get("/image", || async {
            StreamingResponse::new(stream::iter(["x".repeat(LONG)])).media_type("image/png")
        })?
        .get("/partial", || async {
            (StatusCode::PARTIAL_CONTENT, long_text())
        })?
        .get("/not-modified", || async { (StatusCode::NOT_MODIFIED, "") })?
        .get("/encoded", || async {
            let mut response = Response::new(long_text().into());
            response
                .headers_mut()
                .insert(CONTENT_ENCODING, HeaderValue::from_static("br"));
            response
        })?)
}

#[tokio::test]
async fn compresses_a_long_body_for_a_client_that_accepts_gzip() -> TestResult {
    let client = TestClient::start(app(GZipMiddleware::new())?).await?;

    let response = client
        .get("/long")
        .header(ACCEPT_ENCODING, "br, gzip")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_ENCODING], "gzip");
    assert_eq!(response.headers()[VARY], "Accept-Encoding");
    assert_eq!(
        response.headers()[CONTENT_LENGTH],
        response.body().len().to_string()
    );
    assert!(response.body().len() < LONG);
    assert_eq!(decompress(response.body())?, long_text());
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn sends_a_long_body_as_it_is_to_a_client_without_gzip_but_varies_on_it() -> TestResult {
    let client = TestClient::start(app(GZipMiddleware::new())?).await?;

    let identity = client
        .get("/long")
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .await?;
    let without = client.get("/long").send().await?;

    for response in [identity, without] {
        assert!(!response.headers().contains_key(CONTENT_ENCODING));
        assert_eq!(response.headers()[VARY], "Accept-Encoding");
        assert_eq!(response.headers()[CONTENT_LENGTH], LONG.to_string());
        assert_eq!(response.text()?, long_text());
    }
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn sends_a_body_under_the_minimum_size_untouched() -> TestResult {
    let client = TestClient::start(app(GZipMiddleware::new().minimum_size(5000))?).await?;

    for path in ["/short", "/long"] {
        let response = client
            .get(path)
            .header(ACCEPT_ENCODING, "gzip")
            .send()
            .await?;

        assert!(!response.headers().contains_key(CONTENT_ENCODING), "{path}");
        assert!(!response.headers().contains_key(VARY), "{path}");
    }
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn never_encodes_a_response_sent_without_a_body() -> TestResult {
    let client = TestClient::start(app(GZipMiddleware::new().minimum_size(0))?).await?;

    let response = client
        .get("/not-modified")
        .header(ACCEPT_ENCODING, "gzip")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert!(!response.headers().contains_key(CONTENT_ENCODING));
    assert!(!response.headers().contains_key(VARY));
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_head_with_the_length_of_the_compressed_body() -> TestResult {
    let client = TestClient::start(app(GZipMiddleware::new())?).await?;

    let get = client
        .get("/long")
        .header(ACCEPT_ENCODING, "gzip")
        .send()
        .await?;
    let head = client
        .head("/long")
        .header(ACCEPT_ENCODING, "gzip")
        .send()
        .await?;

    assert_eq!(head.headers()[CONTENT_ENCODING], "gzip");
    assert_eq!(head.headers()[CONTENT_LENGTH], get.body().len().to_string());
    assert!(head.body().is_empty());
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn compresses_a_stream_chunk_by_chunk() -> TestResult {
    let client = TestClient::start(app(GZipMiddleware::new().compress_level(1))?).await?;

    let mut streamed = client
        .get("/streamed")
        .header(ACCEPT_ENCODING, "gzip")
        .stream()
        .await?;
    let headers = streamed.headers().clone();
    let mut compressed = Vec::new();
    let mut chunks = 0;
    while let Some(chunk) = streamed.chunk().await {
        compressed.extend_from_slice(&chunk?);
        chunks += 1;
    }
    let identity = client.get("/streamed").send().await?;

    assert_eq!(headers[CONTENT_ENCODING], "gzip");
    assert_eq!(headers[TRANSFER_ENCODING], "chunked");
    assert_eq!(headers[VARY], "Accept-Encoding");
    assert!(!headers.contains_key(CONTENT_LENGTH));
    assert_eq!(chunks, 11);
    assert_eq!(decompress(&compressed)?, "x".repeat(4000));
    assert!(!identity.headers().contains_key(CONTENT_ENCODING));
    assert_eq!(identity.headers()[VARY], "Accept-Encoding");
    assert_eq!(identity.text()?, "x".repeat(4000));
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn leaves_excluded_types_partial_and_encoded_responses_alone() -> TestResult {
    let client = TestClient::start(app(GZipMiddleware::new())?).await?;

    for (path, encoding) in [
        ("/encoded", Some("br")),
        ("/image", None),
        ("/partial", None),
    ] {
        let response = client
            .get(path)
            .header(ACCEPT_ENCODING, "gzip")
            .send()
            .await?;

        assert_eq!(
            response
                .headers()
                .get(CONTENT_ENCODING)
                .map(|value| value.to_str().unwrap_or_default()),
            encoding,
            "{path}"
        );
        assert!(!response.headers().contains_key(VARY), "{path}");
        assert_eq!(response.text()?, long_text(), "{path}");
    }
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn compresses_a_long_body_off_the_async_workers_the_same_way() -> TestResult {
    let client = TestClient::start(app(GZipMiddleware::new().thread_minimum_size(1000))?).await?;

    let whole = client
        .get("/long")
        .header(ACCEPT_ENCODING, "gzip")
        .send()
        .await?;
    let streamed = client
        .get("/streamed")
        .header(ACCEPT_ENCODING, "gzip")
        .send()
        .await?;

    assert_eq!(decompress(whole.body())?, long_text());
    assert_eq!(decompress(streamed.body())?, "x".repeat(4000));
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn sends_a_compressed_body_over_the_wire() -> TestResult {
    let server = RunningServer::start(app(GZipMiddleware::new())?).await?;

    let response = reqwest::Client::new()
        .get(server.url("/long"))
        .header(ACCEPT_ENCODING, "gzip")
        .send()
        .await?;
    let headers = response.headers().clone();
    let body = response.bytes().await?;

    assert_eq!(headers[CONTENT_ENCODING], "gzip");
    assert_eq!(headers[CONTENT_LENGTH], body.len().to_string());
    assert_eq!(decompress(&body)?, long_text());
    server.stop().await
}
