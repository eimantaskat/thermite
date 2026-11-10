use std::borrow::Cow;

use bytes::Bytes;
use serde_json::{Map, Value};

use super::{Content, named_values};
use crate::parse::{MultipartError, PartValue, decode_latin1, parse_multipart, parse_query};
use crate::schema::{Field, Schema};
use crate::upload::UploadedFile;

// a value of a multipart field: text, or a file standing in the form as its marker
enum FormValue {
    Text(String),
    File { marker: Value, has_filename: bool },
}

impl FormValue {
    fn to_value(&self) -> Value {
        match self {
            Self::Text(text) => Value::from(text.as_str()),
            Self::File { marker, .. } => marker.clone(),
        }
    }

    // like FastAPI, an empty value is missing, and so is a file without a filename to a field of files
    fn is_sent(&self, takes_files: bool) -> bool {
        match self {
            Self::Text(text) => !text.is_empty(),
            Self::File { has_filename, .. } => *has_filename || !takes_files,
        }
    }

    // a list keeps its empty text, but drops what a browser sends for a file input left empty
    fn is_kept_in_list(&self, takes_files: bool) -> bool {
        match self {
            Self::Text(_) => true,
            Self::File { has_filename, .. } => *has_filename || !takes_files,
        }
    }
}

// the declared fields as an object, and the files their markers stand for; like Starlette, a body of another content type is an empty form
pub(super) fn form_input(
    schema: &Schema,
    body: &Bytes,
    content: &Content,
) -> Result<(Value, Vec<UploadedFile>), MultipartError> {
    let fields = schema.fields().unwrap_or_default();
    match content {
        Content::Form => {
            let text = decode_latin1(body);
            let pairs = parse_query(&text);
            // like FastAPI, an empty value is missing, though a list keeps its empty items
            let text_reading = Reading {
                to_value: |value: &Cow<'_, str>| Value::from(value.as_ref()),
                is_sent: |value: &Cow<'_, str>, _| !value.is_empty(),
                is_kept: |_: &Cow<'_, str>, _| true,
            };
            Ok((object(fields, &pairs, &text_reading), Vec::new()))
        }
        Content::Multipart { boundary } => {
            let boundary = boundary.as_deref().ok_or(MultipartError::MissingBoundary)?;
            let mut files = Vec::new();
            let pairs: Vec<(Cow<'_, str>, FormValue)> = parse_multipart(body, boundary)?
                .into_iter()
                .map(|(name, value)| {
                    let value = match value {
                        PartValue::Text(text) => FormValue::Text(text),
                        PartValue::File(file) => {
                            let value = FormValue::File {
                                marker: file.marker(files.len()),
                                has_filename: !file.filename().is_empty(),
                            };
                            files.push(file);
                            value
                        }
                    };
                    (Cow::Owned(name), value)
                })
                .collect();
            let part_reading = Reading {
                to_value: FormValue::to_value,
                is_sent: FormValue::is_sent,
                is_kept: FormValue::is_kept_in_list,
            };
            Ok((object(fields, &pairs, &part_reading), files))
        }
        Content::Json | Content::Other => Ok((Value::Object(Map::new()), Vec::new())),
    }
}

// how one kind of form value reads: as JSON, whether it counts as sent alone, and whether a list keeps it, given whether the field takes files
struct Reading<ToValue, IsSent, IsKept> {
    to_value: ToValue,
    is_sent: IsSent,
    is_kept: IsKept,
}

fn object<V>(
    fields: &[Field],
    pairs: &[(Cow<'_, str>, V)],
    reading: &Reading<impl Fn(&V) -> Value, impl Fn(&V, bool) -> bool, impl Fn(&V, bool) -> bool>,
) -> Value {
    let members = fields.iter().filter_map(|field| {
        let takes_files = field.schema().takes_files();
        named_values(
            pairs,
            field,
            &reading.to_value,
            |value| (reading.is_sent)(value, takes_files),
            |value| (reading.is_kept)(value, takes_files),
        )
        .map(|value| (field.name().to_owned(), value))
    });
    Value::Object(members.collect())
}
