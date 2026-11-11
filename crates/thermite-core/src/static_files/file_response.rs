use std::fs::Metadata;
use std::future::ready;
use std::hash::{BuildHasher, RandomState};
use std::io::{self, ErrorKind, SeekFrom};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::{Bytes, BytesMut};
use futures_util::{Stream, StreamExt, stream};
use http::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, HeaderName,
    IF_MODIFIED_SINCE, IF_NONE_MATCH, IF_RANGE, LAST_MODIFIED, RANGE,
};
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, Take};

use super::media_type;
use super::range::{self, RangeRequest};
use crate::body::Body;
use crate::handler::{Request, Response};
use crate::parse::decode_latin1;
use crate::response::plain_text;

const CHUNK_SIZE: usize = 64 * 1024;
// 9999-12-31T23:59:59Z, the last moment an HTTP date can be written for
const LATEST_DATE: u64 = 253_402_300_799;

// Starlette's FileResponse for a file StaticFiles found
pub(super) struct FileResponse {
    // shared by the parts of a multipart answer, each reading the file on its own
    path: Arc<Path>,
    status: StatusCode,
    size: u64,
    media_type: HeaderValue,
    // to the second, as the Last-Modified header carries it
    modified: SystemTime,
    last_modified: String,
    etag: String,
}

impl FileResponse {
    // the ETag needs only to change with the file, so it is the time and size in hex, as nginx writes it
    pub(super) fn new(path: PathBuf, metadata: &Metadata, status: StatusCode) -> Self {
        let since_epoch = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .unwrap_or_default();
        let modified = UNIX_EPOCH + Duration::from_secs(since_epoch.as_secs().min(LATEST_DATE));
        Self {
            media_type: media_type::guess(&path),
            path: Arc::from(path),
            status,
            size: metadata.len(),
            modified,
            last_modified: httpdate::fmt_http_date(modified),
            etag: format!("\"{:x}-{:x}\"", since_epoch.as_nanos(), metadata.len()),
        }
    }

    pub(super) fn respond(self, request: &Request) -> Response {
        let headers = request.headers();
        if self.status == StatusCode::OK && self.is_not_modified(headers) {
            return self.not_modified();
        }
        let head = request.method() == Method::HEAD;
        let range = headers
            .get(RANGE)
            .filter(|_| self.status == StatusCode::OK && self.matches_if_range(headers))
            .map_or(RangeRequest::Whole, |range| {
                range::parse(&decode_latin1(range.as_bytes()), self.size)
            });
        match range {
            RangeRequest::Whole => self.whole(head),
            RangeRequest::Ranges(ranges) => match ranges.as_slice() {
                [range] => self.single(range.clone(), head),
                _ => self.multiple(ranges, head),
            },
            RangeRequest::Malformed(message) => plain_text(StatusCode::BAD_REQUEST, message),
            RangeRequest::NotSatisfiable => {
                let mut response = plain_text(StatusCode::RANGE_NOT_SATISFIABLE, "");
                let size = format!("bytes */{}", self.size);
                insert_text(response.headers_mut(), CONTENT_RANGE, &size);
                response
            }
        }
    }

    // as Starlette 1.10 decides it: any If-None-Match is read before If-Modified-Since
    fn is_not_modified(&self, headers: &HeaderMap) -> bool {
        if let Some(tags) = headers
            .get(IF_NONE_MATCH)
            .map(|tags| decode_latin1(tags.as_bytes()))
            .filter(|tags| !tags.is_empty())
        {
            return tags.trim() == "*"
                || tags.split(',').any(|tag| {
                    let tag = tag.trim();
                    tag.strip_prefix("W/").unwrap_or(tag) == self.etag
                });
        }
        headers
            .get(IF_MODIFIED_SINCE)
            .and_then(|since| since.to_str().ok())
            .and_then(|since| httpdate::parse_http_date(since).ok())
            .is_some_and(|since| since >= self.modified)
    }

    // a weak tag never matches, since a range of a file only like this one would be corrupt
    fn matches_if_range(&self, headers: &HeaderMap) -> bool {
        headers.get(IF_RANGE).is_none_or(|validator| {
            let validator = decode_latin1(validator.as_bytes());
            !validator.starts_with("W/")
                && (validator == self.last_modified || validator == self.etag)
        })
    }

    // only the ETag of what the client holds, as Starlette's NotModifiedResponse keeps
    fn not_modified(&self) -> Response {
        let mut headers = HeaderMap::new();
        insert_text(&mut headers, ETAG, &self.etag);
        response(StatusCode::NOT_MODIFIED, headers, Body::empty())
    }

    fn headers(&self, length: u64) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, self.media_type.clone());
        headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
        insert_text(&mut headers, LAST_MODIFIED, &self.last_modified);
        insert_text(&mut headers, ETAG, &self.etag);
        headers
    }

    fn whole(&self, head: bool) -> Response {
        let body = unless_head(head, || {
            Body::from_stream(file_chunks(Arc::clone(&self.path), 0..self.size))
        });
        response(self.status, self.headers(self.size), body)
    }

    fn single(&self, range: Range<u64>, head: bool) -> Response {
        let mut headers = self.headers(range.end - range.start);
        let content_range = format!("bytes {}-{}/{}", range.start, range.end - 1, self.size);
        insert_text(&mut headers, CONTENT_RANGE, &content_range);
        let body = unless_head(head, || {
            Body::from_stream(file_chunks(Arc::clone(&self.path), range))
        });
        response(StatusCode::PARTIAL_CONTENT, headers, body)
    }

    // each range as a part with its own Content-Type and Content-Range, as Starlette writes them
    fn multiple(&self, ranges: Vec<Range<u64>>, head: bool) -> Response {
        let boundary = boundary();
        let media_type = String::from_utf8_lossy(self.media_type.as_bytes());
        let parts: Vec<_> = ranges
            .into_iter()
            .map(|range| {
                let head = format!(
                    "--{boundary}\r\nContent-Type: {media_type}\r\nContent-Range: bytes {}-{}/{}\r\n\r\n",
                    range.start,
                    range.end - 1,
                    self.size
                );
                (Bytes::from(head), range)
            })
            .collect();
        let closing = Bytes::from(format!("--{boundary}--"));
        let length = parts
            .iter()
            .map(|(head, range)| head.len() as u64 + (range.end - range.start) + 2)
            .sum::<u64>()
            + closing.len() as u64;

        let mut headers = self.headers(length);
        let multipart = format!("multipart/byteranges; boundary={boundary}");
        insert_text(&mut headers, CONTENT_TYPE, &multipart);
        let path = Arc::clone(&self.path);
        let body = unless_head(head, move || {
            let parts = stream::iter(parts).flat_map(move |(head, range)| {
                stream::once(ready(Ok(head)))
                    .chain(file_chunks(Arc::clone(&path), range))
                    .chain(stream::once(ready(Ok(Bytes::from_static(b"\r\n")))))
            });
            Body::from_stream(parts.chain(stream::once(ready(Ok(closing)))))
        });
        response(StatusCode::PARTIAL_CONTENT, headers, body)
    }
}

// a HEAD answer keeps the headers of the GET it stands for, without opening the file
fn unless_head(head: bool, body: impl FnOnce() -> Body) -> Body {
    if head { Body::empty() } else { body() }
}

fn response(status: StatusCode, headers: HeaderMap, body: Body) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

// what goes in is ASCII written here, so it is always a valid header value
fn insert_text(headers: &mut HeaderMap, name: HeaderName, text: &str) {
    if let Ok(value) = HeaderValue::from_str(text) {
        headers.insert(name, value);
    }
}

// 26 hex digits like Starlette's; each RandomState is seeded apart, so no random number crate is needed
fn boundary() -> String {
    let random = || RandomState::new().hash_one(());
    format!("{:016x}{:010x}", random(), random() >> 24)
}

enum Reading {
    Unopened(Arc<Path>, Range<u64>),
    Open(Take<File>),
}

// opened when first read, and read in Starlette's 64 KiB chunks
fn file_chunks(path: Arc<Path>, range: Range<u64>) -> impl Stream<Item = io::Result<Bytes>> + Send {
    stream::try_unfold(Reading::Unopened(path, range), |reading| async move {
        let mut file = match reading {
            Reading::Unopened(_, range) if range.is_empty() => return Ok(None),
            Reading::Unopened(path, range) => {
                let mut file = File::open(&*path).await?;
                file.seek(SeekFrom::Start(range.start)).await?;
                file.take(range.end - range.start)
            }
            Reading::Open(file) => file,
        };
        if file.limit() == 0 {
            return Ok(None);
        }
        let mut chunk = BytesMut::with_capacity(CHUNK_SIZE);
        if file.read_buf(&mut chunk).await? == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "the file is shorter than when it was looked up",
            ));
        }
        Ok(Some((chunk.freeze(), Reading::Open(file))))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn a_boundary_is_26_hex_digits_and_new_each_time() {
        let first = boundary();

        assert_eq!(first.len(), 26);
        assert!(first.bytes().all(|digit| digit.is_ascii_hexdigit()));
        assert_ne!(first, boundary());
    }

    #[tokio::test]
    async fn a_file_cut_short_after_it_was_looked_up_ends_the_body_with_an_error() {
        let path = std::env::temp_dir().join(format!("thermite-cut-short-{}", boundary()));
        fs::write(&path, "0123456789").unwrap();

        let mut chunks = Box::pin(file_chunks(Arc::from(path.as_path()), 0..20));
        let first = chunks.next().await.unwrap().unwrap();
        let error = chunks.next().await.unwrap().unwrap_err();
        fs::remove_file(&path).unwrap();

        assert_eq!(first, "0123456789");
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    }
}
