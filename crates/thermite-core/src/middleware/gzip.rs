use std::io::{self, Write};
use std::mem;

use bytes::Bytes;
use flate2::Compression;
use flate2::write::GzEncoder;
use futures_util::{StreamExt, stream};
use http::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE};
use http::response::Parts;
use http::{HeaderMap, HeaderValue, StatusCode};
use tokio::sync::Semaphore;

use super::vary::append_vary;
use super::{Middleware, Next};
use crate::body::{Body, BoxError};
use crate::handler::{Request, Response};

// Starlette's: formats already compressed, and event streams, which a compressor would hold back
const DEFAULT_EXCLUDED_CONTENT_TYPES: [&str; 14] = [
    "application/grpc",
    "application/gzip",
    "application/x-gzip",
    "application/zip",
    "audio/*",
    "font/woff",
    "font/woff2",
    "image/avif",
    "image/gif",
    "image/jpeg",
    "image/png",
    "image/webp",
    "text/event-stream",
    "video/*",
];
const GZIP: HeaderValue = HeaderValue::from_static("gzip");
// Starlette's limit, so many large responses at once do not each take a blocking thread
static COMPRESSING_THREADS: Semaphore = Semaphore::const_new(40);

// Starlette's GZipMiddleware: compresses a response for a client that accepts gzip
#[derive(Debug, Clone)]
pub struct GZipMiddleware {
    minimum_size: usize,
    level: Compression,
    thread_minimum_size: usize,
    exclude_content_types: Vec<String>,
}

impl GZipMiddleware {
    // Starlette's defaults: bodies from 500 bytes, at level 9, chunks from 128 KiB compressed off the async workers
    #[must_use]
    pub fn new() -> Self {
        Self {
            minimum_size: 500,
            level: Compression::best(),
            thread_minimum_size: 128 * 1024,
            exclude_content_types: DEFAULT_EXCLUDED_CONTENT_TYPES.map(str::to_owned).to_vec(),
        }
    }

    // a whole body shorter than this is sent as it is; a streamed one is always compressed
    #[must_use]
    pub fn minimum_size(mut self, bytes: usize) -> Self {
        self.minimum_size = bytes;
        self
    }

    // from 0, stored, to 9, smallest; a higher level counts as 9
    #[must_use]
    pub fn compress_level(mut self, level: u32) -> Self {
        self.level = Compression::new(level.min(9));
        self
    }

    // a body or chunk this long is compressed on a blocking thread, so it holds up no other request
    #[must_use]
    pub fn thread_minimum_size(mut self, bytes: usize) -> Self {
        self.thread_minimum_size = bytes;
        self
    }

    // in place of the defaults; a parameter such as `; charset` is ignored, and `audio/*` matches any audio type
    #[must_use]
    pub fn exclude_content_types<ContentType: AsRef<str>>(
        mut self,
        content_types: impl IntoIterator<Item = ContentType>,
    ) -> Self {
        self.exclude_content_types = content_types
            .into_iter()
            .map(|content_type| media_type(content_type.as_ref()))
            .collect();
        self
    }

    // a status sent without a body too, since a cache would merge a 304's encoding into what it stored
    fn passes_through(&self, response: &Response) -> bool {
        let status = response.status();
        response.headers().contains_key(CONTENT_ENCODING)
            || status == StatusCode::PARTIAL_CONTENT
            || status.is_informational()
            || status == StatusCode::NO_CONTENT
            || status == StatusCode::NOT_MODIFIED
            || self.excludes(response.headers())
    }

    fn excludes(&self, headers: &HeaderMap) -> bool {
        let content_type = headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let media_type = media_type(content_type);
        let (major, _) = media_type.split_once('/').unwrap_or((&media_type, ""));
        let any_of_major = format!("{major}/*");
        let grpc = media_type
            .starts_with("application/grpc+")
            .then_some("application/grpc");
        self.exclude_content_types.iter().any(|excluded| {
            *excluded == media_type || *excluded == any_of_major || Some(excluded.as_str()) == grpc
        })
    }

    async fn compress_whole(&self, mut parts: Parts, whole: Bytes, accepts_gzip: bool) -> Response {
        if whole.len() < self.minimum_size {
            return Response::from_parts(parts, Body::from(whole));
        }
        append_vary(&mut parts.headers, "Accept-Encoding");
        if !accepts_gzip {
            return Response::from_parts(parts, Body::from(whole));
        }
        let level = self.level;
        let off_thread = whole.len() >= self.thread_minimum_size;
        let uncompressed = whole.clone();
        match compress(off_thread, move || compress_body(&uncompressed, level)).await {
            Ok(compressed) => {
                parts.headers.insert(CONTENT_ENCODING, GZIP);
                parts
                    .headers
                    .insert(CONTENT_LENGTH, HeaderValue::from(compressed.len()));
                Response::from_parts(parts, Body::from(compressed))
            }
            // compressing into memory does not fail, so this only keeps the response if it ever did
            Err(error) => {
                tracing::error!("could not compress a response, so it is sent as it is: {error}");
                Response::from_parts(parts, Body::from(whole))
            }
        }
    }

    fn compress_stream(&self, mut parts: Parts, body: Body, accepts_gzip: bool) -> Response {
        append_vary(&mut parts.headers, "Accept-Encoding");
        if !accepts_gzip {
            return Response::from_parts(parts, body);
        }
        parts.headers.insert(CONTENT_ENCODING, GZIP);
        parts.headers.remove(CONTENT_LENGTH);
        let body = Body::from_stream(compressed_chunks(
            body,
            self.level,
            self.thread_minimum_size,
        ));
        Response::from_parts(parts, body)
    }
}

impl Default for GZipMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl Middleware for GZipMiddleware {
    async fn call(&self, request: Request, next: Next) -> Response {
        // as Starlette reads it, so `gzip;q=0` still counts
        let accepts_gzip = request
            .headers()
            .get(ACCEPT_ENCODING)
            .is_some_and(|value| value.as_bytes().windows(4).any(|word| word == b"gzip"));
        let response = next.run(request).await;
        if self.passes_through(&response) {
            return response;
        }
        let (parts, body) = response.into_parts();
        match body.as_bytes().cloned() {
            Some(whole) => self.compress_whole(parts, whole, accepts_gzip).await,
            None => self.compress_stream(parts, body, accepts_gzip),
        }
    }
}

fn media_type(content_type: &str) -> String {
    let (media_type, _) = content_type.split_once(';').unwrap_or((content_type, ""));
    media_type.trim().to_ascii_lowercase()
}

async fn compress<T: Send + 'static>(
    off_thread: bool,
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> Result<T, BoxError> {
    if off_thread {
        let _permit = COMPRESSING_THREADS.acquire().await?;
        Ok(tokio::task::spawn_blocking(work).await??)
    } else {
        Ok(work()?)
    }
}

fn compress_body(body: &[u8], level: Compression) -> io::Result<Bytes> {
    let mut encoder = GzEncoder::new(Vec::new(), level);
    encoder.write_all(body)?;
    Ok(Bytes::from(encoder.finish()?))
}

// each chunk flushed, so the client can decode what has come so far, as Starlette's sync flush does
fn compressed_chunks(
    body: Body,
    level: Compression,
    thread_minimum_size: usize,
) -> impl futures_util::Stream<Item = Result<Bytes, BoxError>> + Send {
    let encoder = GzEncoder::new(Vec::new(), level);
    stream::unfold(Some((body, encoder)), move |state| async move {
        let (mut body, mut encoder) = state?;
        match body.next().await {
            Some(Ok(chunk)) => {
                let off_thread = chunk.len() >= thread_minimum_size;
                let compressed = compress(off_thread, move || {
                    encoder.write_all(&chunk)?;
                    encoder.flush()?;
                    let compressed = Bytes::from(mem::take(encoder.get_mut()));
                    Ok((compressed, encoder))
                })
                .await;
                Some(match compressed {
                    Ok((compressed, encoder)) => (Ok(compressed), Some((body, encoder))),
                    Err(error) => (Err(error), None),
                })
            }
            Some(Err(error)) => Some((Err(error), None)),
            None => Some((encoder.finish().map(Bytes::from).map_err(Into::into), None)),
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::Read;

    use flate2::read::GzDecoder;

    use super::*;

    fn decompress(compressed: &[u8]) -> String {
        let mut text = String::new();
        GzDecoder::new(compressed)
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    fn with_content_type(content_type: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
        headers
    }

    #[test]
    fn excludes_a_content_type_by_its_media_type_its_major_type_or_grpc() {
        let gzip = GZipMiddleware::new();

        for excluded in [
            "image/PNG; q=1",
            "audio/ogg",
            "application/grpc+proto",
            "text/event-stream",
        ] {
            assert!(gzip.excludes(&with_content_type(excluded)), "{excluded}");
        }
        assert!(!gzip.excludes(&with_content_type("text/html")));
        assert!(!gzip.excludes(&HeaderMap::new()));
        let only_csv = gzip.exclude_content_types(["Text/CSV; charset=utf-8"]);
        assert!(only_csv.excludes(&with_content_type("text/csv")));
        assert!(!only_csv.excludes(&with_content_type("image/png")));
    }

    #[test]
    fn a_level_over_9_counts_as_9() {
        let gzip = GZipMiddleware::default().compress_level(42);

        assert_eq!(gzip.level, Compression::best());
        assert_eq!(
            GZipMiddleware::new().compress_level(1).level,
            Compression::fast()
        );
    }

    #[tokio::test]
    async fn compresses_off_the_async_workers_from_the_thread_minimum_size() {
        let whole = Bytes::from("x".repeat(64));

        let compressed = compress(true, move || compress_body(&whole, Compression::best()))
            .await
            .unwrap();

        assert_eq!(decompress(&compressed), "x".repeat(64));
    }

    #[tokio::test]
    async fn each_compressed_chunk_decodes_to_what_came_so_far() {
        let chunks = stream::iter(["first ", "second"].map(Ok::<_, io::Error>));
        let mut compressed = Body::from_stream(compressed_chunks(
            Body::from_stream(chunks),
            Compression::best(),
            6,
        ));

        let first = compressed.chunk().await.unwrap().unwrap();
        let mut all = first.to_vec();
        while let Some(chunk) = compressed.chunk().await {
            all.extend_from_slice(&chunk.unwrap());
        }

        let mut so_far = String::new();
        let _ = GzDecoder::new(&first[..]).read_to_string(&mut so_far);
        assert_eq!(so_far, "first ");
        assert_eq!(decompress(&all), "first second");
    }

    #[tokio::test]
    async fn an_error_in_the_stream_ends_the_compressed_one() {
        let chunks = stream::iter([Ok("first"), Err(io::Error::other("lost"))]);
        let mut compressed = Body::from_stream(compressed_chunks(
            Body::from_stream(chunks),
            Compression::best(),
            usize::MAX,
        ));

        assert!(compressed.chunk().await.unwrap().is_ok());
        let error = compressed.chunk().await.unwrap().unwrap_err();
        assert_eq!(error.to_string(), "lost");
        assert!(compressed.chunk().await.is_none());
    }
}
