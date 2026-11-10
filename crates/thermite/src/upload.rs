use std::cell::RefCell;

use bytes::Bytes;
use http::HeaderMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use thermite_core::{Schema, UploadedFile, UploadedFiles, Value};

use crate::model::ToSchema;

// FastAPI's UploadFile: a file part of a multipart form, read whole into memory
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadFile(UploadedFile);

impl UploadFile {
    #[must_use]
    pub fn filename(&self) -> &str {
        self.0.filename()
    }

    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.0.content_type()
    }

    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        self.0.headers()
    }

    #[must_use]
    pub fn content(&self) -> &Bytes {
        self.0.content()
    }

    #[must_use]
    pub fn into_content(self) -> Bytes {
        self.0.into_content()
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.0.size()
    }
}

impl ToSchema for UploadFile {
    fn schema() -> Schema {
        Schema::file()
    }
}

thread_local! {
    static FILES: RefCell<Option<UploadedFiles>> = const { RefCell::new(None) };
}

// serde hands a type nothing but the value, so the files a form's markers stand for are lent to it on this thread
pub(crate) fn reading_files<R>(files: &UploadedFiles, read: impl FnOnce() -> R) -> R {
    if files.is_empty() {
        return read();
    }
    let _lent = Lent::start(files.clone());
    read()
}

struct Lent {
    previous: Option<UploadedFiles>,
}

impl Lent {
    fn start(files: UploadedFiles) -> Self {
        Self {
            previous: FILES.with_borrow_mut(|lent| lent.replace(files)),
        }
    }
}

// given back even when deserializing panics, so no request's files stay on the thread
impl Drop for Lent {
    fn drop(&mut self) {
        let previous = self.previous.take();
        FILES.with_borrow_mut(|lent| *lent = previous);
    }
}

impl<'de> Deserialize<'de> for UploadFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let marker = Value::deserialize(deserializer)?;
        FILES
            .with_borrow(|files| files.as_ref()?.find(&marker).cloned())
            .map(Self)
            .ok_or_else(|| D::Error::custom("an UploadFile is read only from a form's files"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn is_read_only_from_the_files_a_form_lends() {
        let marker = json!({"filename": "a.txt", "content_type": null, "size": 0, "index": 0});

        let outside = UploadFile::deserialize(&marker);
        let unlent = reading_files(&UploadedFiles::default(), || {
            UploadFile::deserialize(&marker)
        });

        assert!(outside.is_err());
        assert!(unlent.is_err());
        assert_eq!(UploadFile::schema(), Schema::file());
    }
}
