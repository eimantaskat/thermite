use std::convert::Infallible;

use bytes::Bytes;
use futures_util::FutureExt;
use http::header::CONTENT_TYPE;

use super::decode_latin1;
use crate::upload::UploadedFile;

// Starlette's defaults for `request.form()`, which FastAPI keeps
const MAX_FILES: usize = 1000;
const MAX_FIELDS: usize = 1000;
const MAX_PART_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PartValue {
    Text(String),
    File(UploadedFile),
}

// each answers 400 with Starlette's message
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum MultipartError {
    #[error("Missing boundary in multipart.")]
    MissingBoundary,
    #[error("Invalid multipart data.")]
    Invalid,
    #[error("The Content-Disposition header field \"name\" must be provided.")]
    MissingName,
    #[error("Too many files. Maximum number of files is {MAX_FILES}.")]
    TooManyFiles,
    #[error("Too many fields. Maximum number of fields is {MAX_FIELDS}.")]
    TooManyFields,
    #[error("Part exceeded maximum size of {}KB.", MAX_PART_SIZE / 1024)]
    PartTooLarge,
}

// the body is already read, so the parser never waits and is driven without a runtime
pub(crate) fn parse_multipart(
    body: &Bytes,
    boundary: &str,
) -> Result<Vec<(String, PartValue)>, MultipartError> {
    // like Starlette, an empty body is an empty form
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let stream = futures_util::stream::iter([Ok::<_, Infallible>(body.clone())]);
    let mut multipart = multer::Multipart::new(stream, boundary);
    let mut parts = Vec::new();
    let (mut files, mut fields) = (0, 0);
    while let Some(field) = ready(multipart.next_field())? {
        let name = field.name().ok_or(MultipartError::MissingName)?.to_owned();
        let filename = field.file_name().map(str::to_owned);
        let headers = filename.as_ref().map(|_| field.headers().clone());
        if filename.is_some() {
            files += 1;
            if files > MAX_FILES {
                return Err(MultipartError::TooManyFiles);
            }
        } else {
            fields += 1;
            if fields > MAX_FIELDS {
                return Err(MultipartError::TooManyFields);
            }
        }
        let content = ready(field.bytes())?;
        let value = match filename.zip(headers) {
            Some((filename, headers)) => {
                let content_type = headers
                    .get(CONTENT_TYPE)
                    .map(|value| decode_latin1(value.as_bytes()).into_owned());
                PartValue::File(UploadedFile::new(filename, content_type, headers, content))
            }
            None if content.len() > MAX_PART_SIZE => return Err(MultipartError::PartTooLarge),
            None => PartValue::Text(decode_text(&content)),
        };
        parts.push((name, value));
    }
    Ok(parts)
}

fn ready<T>(future: impl Future<Output = multer::Result<T>>) -> Result<T, MultipartError> {
    match future.now_or_never() {
        Some(Ok(value)) => Ok(value),
        Some(Err(error)) => {
            // the client gets Starlette's message, the log what multer found
            tracing::debug!(%error, "invalid multipart body");
            Err(MultipartError::Invalid)
        }
        // multer waits only on the stream, which holds the whole body and then ends
        None => Err(MultipartError::Invalid),
    }
}

// like Starlette, text that is not UTF-8 is read as latin-1
fn decode_text(content: &[u8]) -> String {
    match std::str::from_utf8(content) {
        Ok(text) => text.to_owned(),
        Err(_) => decode_latin1(content).into_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const BOUNDARY: &str = "XyZ";

    fn body(parts: &[(&[&str], &[u8])]) -> Bytes {
        let mut body = Vec::new();
        for (headers, content) in parts {
            body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
            for header in *headers {
                body.extend_from_slice(format!("{header}\r\n").as_bytes());
            }
            body.extend_from_slice(b"\r\n");
            body.extend_from_slice(content);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
        Bytes::from(body)
    }

    fn text(name: &str) -> String {
        format!("Content-Disposition: form-data; name=\"{name}\"")
    }

    fn file(name: &str, filename: &str) -> String {
        format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"")
    }

    fn as_file(value: &PartValue) -> Option<&UploadedFile> {
        match value {
            PartValue::File(file) => Some(file),
            PartValue::Text(_) => None,
        }
    }

    fn parse(parts: &[(&[&str], &[u8])]) -> Result<Vec<(String, PartValue)>, MultipartError> {
        parse_multipart(&body(parts), BOUNDARY)
    }

    #[test]
    fn reads_text_fields_and_files_in_order() {
        let (title, upload) = (text("title"), file("file", "a.txt"));
        let parts = parse(&[
            (&[&title], b"hello"),
            (
                &[&upload, "Content-Type: text/plain", "X-Custom: 1"],
                b"line\r\nmore",
            ),
        ])
        .unwrap();

        assert_eq!(
            parts[0],
            ("title".to_owned(), PartValue::Text("hello".to_owned()))
        );
        assert_eq!(parts[1].0, "file");
        let file = as_file(&parts[1].1).unwrap();
        assert_eq!(file.filename(), "a.txt");
        assert_eq!(file.content_type(), Some("text/plain"));
        assert_eq!(file.headers()["x-custom"], "1");
        assert_eq!(file.content().as_ref(), b"line\r\nmore");
    }

    #[test]
    fn a_file_without_a_content_type_has_none() {
        let upload = file("file", "a.bin");
        let parts = parse(&[(&[&upload], b"x")]).unwrap();
        assert_eq!(as_file(&parts[0].1).unwrap().content_type(), None);
    }

    #[test]
    fn reads_text_as_utf8_and_falls_back_to_latin1() {
        let name = text("name");
        let parts = parse(&[(&[&name], "žė".as_bytes()), (&[&name], b"\xff\xfe")]).unwrap();
        assert_eq!(parts[0].1, PartValue::Text("žė".to_owned()));
        assert_eq!(parts[1].1, PartValue::Text("ÿþ".to_owned()));
    }

    #[test]
    fn an_empty_body_is_an_empty_form() {
        assert_eq!(parse_multipart(&Bytes::new(), BOUNDARY), Ok(Vec::new()));
    }

    #[test]
    fn refuses_a_part_without_a_name() {
        for headers in [&["X-A: 1"][..], &["Content-Disposition: form-data"][..]] {
            assert_eq!(parse(&[(headers, b"v")]), Err(MultipartError::MissingName));
        }
    }

    #[test]
    fn refuses_a_body_that_is_not_multipart() {
        let name = text("name");
        let truncated = body(&[(&[&name], b"n")]).slice(..20);
        for body in [Bytes::from_static(b"garbage"), truncated] {
            assert_eq!(
                parse_multipart(&body, BOUNDARY),
                Err(MultipartError::Invalid),
                "{body:?}"
            );
        }
    }

    #[test]
    fn allows_as_many_files_and_fields_as_starlette() {
        let (name, upload) = (text("name"), file("file", "a"));
        let (name, upload) = ([name.as_str()], [upload.as_str()]);
        let mut parts: Vec<(&[&str], &[u8])> = vec![(&name, b"n"); MAX_FIELDS];
        parts.extend(vec![(&upload[..], &b""[..]); MAX_FILES]);
        assert_eq!(parse(&parts).unwrap().len(), MAX_FIELDS + MAX_FILES);

        parts.push((&name, b"n"));
        assert_eq!(parse(&parts), Err(MultipartError::TooManyFields));
        parts.pop();
        parts.push((&upload, b""));
        assert_eq!(parse(&parts), Err(MultipartError::TooManyFiles));
    }

    #[test]
    fn limits_the_size_of_a_text_field_but_not_of_a_file() {
        let (name, upload) = (text("name"), file("file", "a"));
        let largest = vec![b'a'; MAX_PART_SIZE];
        let larger = vec![b'a'; MAX_PART_SIZE + 1];
        assert!(parse(&[(&[&name], &largest), (&[&upload], &larger)]).is_ok());
        assert_eq!(
            parse(&[(&[&name], &larger)]),
            Err(MultipartError::PartTooLarge)
        );
    }

    #[test]
    fn says_what_starlette_says() {
        assert_eq!(
            MultipartError::TooManyFiles.to_string(),
            "Too many files. Maximum number of files is 1000."
        );
        assert_eq!(
            MultipartError::PartTooLarge.to_string(),
            "Part exceeded maximum size of 1024KB."
        );
    }

    proptest! {
        #[test]
        fn reads_back_any_text_it_was_sent(values in proptest::collection::vec("[^\r]{0,40}", 0..8)) {
            let name = text("name");
            let headers = [name.as_str()];
            let parts: Vec<(&[&str], &[u8])> =
                values.iter().map(|value| (&headers[..], value.as_bytes())).collect();
            let read: Vec<PartValue> = parse(&parts).unwrap().into_iter().map(|(_, value)| value).collect();

            prop_assert_eq!(read, values.into_iter().map(PartValue::Text).collect::<Vec<_>>());
        }

        #[test]
        fn never_panics_on_any_body(body in proptest::collection::vec(any::<u8>(), 0..256)) {
            let _ = parse_multipart(&Bytes::from(body), BOUNDARY);
        }
    }
}
