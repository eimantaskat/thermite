use bytes::Bytes;
use http::HeaderValue;

// a long fixed boundary, as browsers use, which only a body made to hold it would contain
const BOUNDARY: &str = "thermite-test-boundary-0c6e5b3f9a7d41e2";
const CONTENT_TYPE: &str = "multipart/form-data; boundary=thermite-test-boundary-0c6e5b3f9a7d41e2";

// a multipart form for `TestRequest::multipart`, like httpx's `data` and `files`
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestMultipart {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Part {
    name: String,
    file: Option<(String, String)>,
    content: Bytes,
}

impl TestMultipart {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn text(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.parts.push(Part {
            name: name.into(),
            file: None,
            content: Bytes::from(value.into()),
        });
        self
    }

    #[must_use]
    pub fn file(
        mut self,
        name: impl Into<String>,
        filename: impl Into<String>,
        content_type: impl Into<String>,
        content: impl Into<Bytes>,
    ) -> Self {
        self.parts.push(Part {
            name: name.into(),
            file: Some((filename.into(), content_type.into())),
            content: content.into(),
        });
        self
    }

    pub(super) fn encode(&self) -> (HeaderValue, Bytes) {
        let mut body = Vec::new();
        for part in &self.parts {
            body.extend_from_slice(b"--");
            body.extend_from_slice(BOUNDARY.as_bytes());
            body.extend_from_slice(b"\r\nContent-Disposition: form-data; name=\"");
            quote(&mut body, &part.name);
            body.push(b'"');
            if let Some((filename, content_type)) = &part.file {
                body.extend_from_slice(b"; filename=\"");
                quote(&mut body, filename);
                body.extend_from_slice(b"\"\r\nContent-Type: ");
                quote(&mut body, content_type);
            }
            body.extend_from_slice(b"\r\n\r\n");
            body.extend_from_slice(&part.content);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(b"--");
        body.extend_from_slice(BOUNDARY.as_bytes());
        body.extend_from_slice(b"--\r\n");
        (HeaderValue::from_static(CONTENT_TYPE), Bytes::from(body))
    }
}

// escaped as browsers and httpx do, so a quote or a line break cannot end a header or its value
fn quote(body: &mut Vec<u8>, text: &str) {
    for byte in text.bytes() {
        match byte {
            b'"' => body.extend_from_slice(b"%22"),
            b'\r' => body.extend_from_slice(b"%0D"),
            b'\n' => body.extend_from_slice(b"%0A"),
            byte => body.push(byte),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_each_part_between_boundaries() {
        let (content_type, body) = TestMultipart::new()
            .text("title", "hi")
            .file("file", "a\"b.txt", "text/plain", "hello")
            .encode();

        assert!(CONTENT_TYPE.ends_with(BOUNDARY));
        assert_eq!(content_type, CONTENT_TYPE);
        assert_eq!(
            body,
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nhi\r\n\
                 --{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a%22b.txt\"\r\n\
                 Content-Type: text/plain\r\n\r\nhello\r\n--{BOUNDARY}--\r\n"
            )
        );
    }

    #[test]
    fn escapes_line_breaks_in_names() {
        let (_, body) = TestMultipart::new()
            .text("a\r\nb", "")
            .file("f", "f", "text/plain\r\nX: 1", "")
            .encode();

        assert!(body.windows(9).any(|window| window == b"a%0D%0Ab\""));
        assert!(body.windows(16).any(|window| window == b"text/plain%0D%0A"));
    }
}
