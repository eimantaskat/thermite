use std::sync::Arc;

use bytes::Bytes;
use http::HeaderMap;
use serde_json::{Map, Value};

// a file part of a multipart body, kept in memory
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedFile {
    filename: String,
    content_type: Option<String>,
    headers: HeaderMap,
    content: Bytes,
}

// the files of one request; a form's validated value names each by the marker it holds in its place
// most requests hold none, which `None` keeps free of the shared counter even an empty `Arc` touches
#[derive(Debug, Clone, Default)]
pub struct UploadedFiles(Option<Arc<[UploadedFile]>>);

// the key of a marker that holds the file's position, beside what an error shows of the file
const INDEX: &str = "index";

impl UploadedFile {
    pub(crate) fn new(
        filename: String,
        content_type: Option<String>,
        headers: HeaderMap,
        content: Bytes,
    ) -> Self {
        Self {
            filename,
            content_type,
            headers,
            content,
        }
    }

    #[must_use]
    pub fn filename(&self) -> &str {
        &self.filename
    }

    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    #[must_use]
    pub fn content(&self) -> &Bytes {
        &self.content
    }

    #[must_use]
    pub fn into_content(self) -> Bytes {
        self.content
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.content.len()
    }

    // what a form's value holds in place of the file
    pub(crate) fn marker(&self, index: usize) -> Value {
        let mut marker = Map::new();
        marker.insert("filename".to_owned(), Value::from(self.filename.as_str()));
        marker.insert(
            "content_type".to_owned(),
            Value::from(self.content_type.as_deref()),
        );
        marker.insert("size".to_owned(), Value::from(self.size()));
        marker.insert(INDEX.to_owned(), Value::from(index));
        Value::Object(marker)
    }
}

impl UploadedFiles {
    pub(crate) fn new(files: Vec<UploadedFile>) -> Self {
        Self((!files.is_empty()).then(|| files.into()))
    }

    // the file a marker stands for
    #[must_use]
    pub fn find(&self, marker: &Value) -> Option<&UploadedFile> {
        let index = marker_index(marker)?;
        self.0.as_deref()?.get(usize::try_from(index).ok()?)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }
}

// a form's text values are strings, so an object in one can only be a marker
pub(crate) fn is_marker(value: &Value) -> bool {
    marker_index(value).is_some()
}

fn marker_index(value: &Value) -> Option<u64> {
    value.as_object()?.get(INDEX)?.as_u64()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn file(filename: &str) -> UploadedFile {
        UploadedFile::new(
            filename.to_owned(),
            Some("text/plain".to_owned()),
            HeaderMap::new(),
            Bytes::from_static(b"hello"),
        )
    }

    #[test]
    fn a_marker_shows_the_file_and_finds_it_again() {
        let files = UploadedFiles::new(vec![file("a.txt"), file("b.txt")]);
        let marker = file("b.txt").marker(1);
        assert_eq!(
            marker,
            json!({"filename": "b.txt", "content_type": "text/plain", "size": 5, "index": 1})
        );
        assert!(is_marker(&marker));
        assert_eq!(
            files.find(&marker).map(UploadedFile::filename),
            Some("b.txt")
        );
        assert!(!files.is_empty());
    }

    #[test]
    fn finds_no_file_for_anything_but_a_marker_of_one() {
        let files = UploadedFiles::new(vec![file("a.txt")]);
        for value in [
            json!("a.txt"),
            json!({}),
            json!({"index": "0"}),
            json!({"index": 1}),
        ] {
            assert_eq!(files.find(&value), None, "{value}");
        }
        assert!(!is_marker(&json!({"index": -1})));
        assert!(UploadedFiles::new(Vec::new()).is_empty());
    }

    #[test]
    fn gives_out_what_the_part_held() {
        let file = file("a.txt");
        assert_eq!(file.filename(), "a.txt");
        assert_eq!(file.content_type(), Some("text/plain"));
        assert!(file.headers().is_empty());
        assert_eq!(file.content(), &Bytes::from_static(b"hello"));
        assert_eq!(file.size(), 5);
        assert_eq!(file.into_content(), Bytes::from_static(b"hello"));
    }
}
