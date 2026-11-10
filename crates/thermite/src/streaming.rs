use std::convert::Infallible;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use http::header::CONTENT_TYPE;
use http::{HeaderValue, StatusCode};
use thermite_core::{Body, BoxError, Response, ResponseDescription};

use crate::response::{IntoResponse, SUCCESSFUL_RESPONSE, internal_server_error};

// Starlette's StreamingResponse: a body written chunk by chunk as the stream produces it
#[derive(Debug)]
pub struct StreamingResponse {
    body: Body,
    media_type: Option<String>,
}

impl StreamingResponse {
    pub fn new<Chunks>(chunks: Chunks) -> Self
    where
        Chunks: Stream + Send + 'static,
        Chunks::Item: Into<Bytes>,
    {
        Self::from_results(chunks.map(Ok::<_, Infallible>))
    }

    // an error ends the response early, closing the connection, as an exception does in Starlette once the status is sent
    pub fn from_results<Chunks, Chunk, Error>(chunks: Chunks) -> Self
    where
        Chunks: Stream<Item = Result<Chunk, Error>> + Send + 'static,
        Chunk: Into<Bytes>,
        Error: Into<BoxError>,
    {
        Self {
            body: Body::from_stream(chunks),
            media_type: None,
        }
    }

    #[must_use]
    pub fn media_type(mut self, media_type: impl Into<String>) -> Self {
        self.media_type = Some(media_type.into());
        self
    }
}

impl IntoResponse for StreamingResponse {
    fn describe() -> Vec<ResponseDescription> {
        vec![ResponseDescription::new(
            StatusCode::OK,
            SUCCESSFUL_RESPONSE,
        )]
    }

    fn into_response(self) -> Response {
        let mut response = Response::new(self.body);
        if let Some(media_type) = self.media_type {
            match HeaderValue::try_from(with_charset(media_type)) {
                Ok(content_type) => {
                    response.headers_mut().insert(CONTENT_TYPE, content_type);
                }
                Err(error) => {
                    tracing::error!(
                        "a streaming response's media type is not a header value: {error}"
                    );
                    return internal_server_error();
                }
            }
        }
        response
    }
}

// as Starlette writes the content type of a text media type
fn with_charset(media_type: String) -> String {
    if media_type.starts_with("text/") && !media_type.to_ascii_lowercase().contains("charset=") {
        media_type + "; charset=utf-8"
    } else {
        media_type
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use futures_util::stream;

    use super::*;

    #[tokio::test]
    async fn streams_each_chunk_under_its_media_type() {
        let response = StreamingResponse::new(stream::iter(["a,b\n", "1,2\n"]))
            .media_type("text/csv")
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "text/csv; charset=utf-8");
        assert!(response.body().is_stream());
        assert_eq!(
            response.into_body().into_bytes().await.unwrap(),
            "a,b\n1,2\n"
        );
    }

    #[test]
    fn writes_the_charset_only_of_a_text_type_without_one() {
        assert_eq!(
            with_charset("text/plain; charset=latin-1".to_owned()),
            "text/plain; charset=latin-1"
        );
        assert_eq!(
            with_charset("text/plain; Charset=latin-1".to_owned()),
            "text/plain; Charset=latin-1"
        );
        assert_eq!(
            with_charset("application/x-ndjson".to_owned()),
            "application/x-ndjson"
        );
        let unset = StreamingResponse::new(stream::empty::<Bytes>()).into_response();
        assert!(!unset.headers().contains_key(CONTENT_TYPE));
    }

    #[test]
    fn a_media_type_that_is_no_header_value_answers_500() {
        let response = StreamingResponse::new(stream::empty::<Bytes>())
            .media_type("text/csv\n")
            .into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            StreamingResponse::describe(),
            [ResponseDescription::new(
                StatusCode::OK,
                SUCCESSFUL_RESPONSE
            )]
        );
    }
}
