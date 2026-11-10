use std::fmt::{self, Write as _};
use std::sync::{Arc, OnceLock};

use http::StatusCode;
use serde_json::{Value, json};

use crate::handler::Response;
use crate::response::{attach_error, json_response};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorKind {
    Missing,
    StringType,
    IntType,
    IntParsing,
    IntFromFloat,
    IntParsingSize,
    FloatType,
    FloatParsing,
    FiniteNumber,
    BoolType,
    BoolParsing,
    ListType,
    ModelAttributesType,
    JsonInvalid,
    UploadFileType,
}

impl ErrorKind {
    // the same `type` and `msg` pairs that pydantic reports
    fn type_and_message(self) -> (&'static str, &'static str) {
        match self {
            Self::Missing => ("missing", "Field required"),
            Self::StringType => ("string_type", "Input should be a valid string"),
            Self::IntType => ("int_type", "Input should be a valid integer"),
            Self::IntParsing => (
                "int_parsing",
                "Input should be a valid integer, unable to parse string as an integer",
            ),
            Self::IntFromFloat => (
                "int_from_float",
                "Input should be a valid integer, got a number with a fractional part",
            ),
            Self::IntParsingSize => (
                "int_parsing_size",
                "Unable to parse input string as an integer, exceeded maximum size",
            ),
            Self::FloatType => ("float_type", "Input should be a valid number"),
            Self::FloatParsing => (
                "float_parsing",
                "Input should be a valid number, unable to parse string as a number",
            ),
            Self::FiniteNumber => ("finite_number", "Input should be a finite number"),
            Self::BoolType => ("bool_type", "Input should be a valid boolean"),
            Self::BoolParsing => (
                "bool_parsing",
                "Input should be a valid boolean, unable to interpret input",
            ),
            Self::ListType => ("list_type", "Input should be a valid list"),
            Self::ModelAttributesType => (
                "model_attributes_type",
                "Input should be a valid dictionary or object to extract fields from",
            ),
            Self::JsonInvalid => ("json_invalid", "JSON decode error"),
            Self::UploadFileType => (
                "value_error",
                "Value error, Expected UploadFile, received: <class 'str'>",
            ),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum LocationItem<'text> {
    Source(&'static str),
    Key(&'text Arc<str>),
    Index(usize),
}

// what an error keeps of its location: sources and names shared, never copied into strings
#[derive(Debug, Clone)]
enum Segment {
    Source(&'static str),
    Key(Arc<str>),
    Index(usize),
}

impl Segment {
    fn key(&self) -> Option<&str> {
        match self {
            Self::Source(source) => Some(source),
            Self::Key(key) => Some(key),
            Self::Index(_) => None,
        }
    }

    fn index(&self) -> Option<usize> {
        match self {
            Self::Index(index) => Some(*index),
            Self::Source(_) | Self::Key(_) => None,
        }
    }

    fn to_json(&self) -> Value {
        match self.key() {
            Some(key) => Value::from(key),
            None => Value::from(self.index()),
        }
    }
}

// a source and a key of the same text are the same `loc` entry
impl PartialEq for Segment {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key() && self.index() == other.index()
    }
}

impl Eq for Segment {}

impl fmt::Display for Segment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => write_json_string(formatter, source),
            Self::Key(key) => write_json_string(formatter, key),
            Self::Index(index) => write!(formatter, "{index}"),
        }
    }
}

// escapes the way serde_json does, without the `Value` it needs to write a string
fn write_json_string(formatter: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    formatter.write_char('"')?;
    for character in text.chars() {
        match character {
            '"' => formatter.write_str("\\\"")?,
            '\\' => formatter.write_str("\\\\")?,
            '\u{8}' => formatter.write_str("\\b")?,
            '\u{c}' => formatter.write_str("\\f")?,
            '\n' => formatter.write_str("\\n")?,
            '\r' => formatter.write_str("\\r")?,
            '\t' => formatter.write_str("\\t")?,
            control if control < ' ' => write!(formatter, "\\u{:04x}", u32::from(control))?,
            character => formatter.write_char(character)?,
        }
    }
    formatter.write_char('"')
}

// each level borrows its parent, so nothing is allocated until an error needs the full path
#[derive(Debug)]
pub(crate) struct Location<'parent> {
    parent: Option<&'parent Location<'parent>>,
    item: LocationItem<'parent>,
}

impl<'parent> Location<'parent> {
    pub(crate) fn root(source: &'static str) -> Self {
        Self {
            parent: None,
            item: LocationItem::Source(source),
        }
    }

    pub(crate) fn key<'child>(&'child self, key: &'child Arc<str>) -> Location<'child> {
        Location {
            parent: Some(self),
            item: LocationItem::Key(key),
        }
    }

    pub(crate) fn index(&self, index: usize) -> Location<'_> {
        Location {
            parent: Some(self),
            item: LocationItem::Index(index),
        }
    }

    fn ancestors(&self) -> impl Iterator<Item = &Location<'parent>> {
        std::iter::successors(Some(self), |location| location.parent)
    }

    fn to_segments(&self) -> Vec<Segment> {
        let mut segments = Vec::with_capacity(self.ancestors().count());
        segments.extend(self.ancestors().map(|location| match location.item {
            LocationItem::Source(source) => Segment::Source(source),
            LocationItem::Key(key) => Segment::Key(Arc::clone(key)),
            LocationItem::Index(index) => Segment::Index(index),
        }));
        segments.reverse();
        segments
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidationError {
    kind: ErrorKind,
    location: Vec<Segment>,
    input: Value,
    context: Option<Value>,
}

impl ValidationError {
    pub(crate) fn new(kind: ErrorKind, location: &Location<'_>, input: Value) -> Self {
        Self {
            kind,
            location: location.to_segments(),
            input,
            context: None,
        }
    }

    // takes `loc` as JSON, so tests can write it the way it appears in the response
    #[cfg(test)]
    #[allow(clippy::expect_used)] // only given literal locations
    pub(crate) fn at(kind: ErrorKind, location: Value, input: Value) -> Self {
        let segment = |item: Value| match item {
            Value::String(key) => Segment::Key(Arc::from(key)),
            index => Segment::Index(serde_json::from_value(index).expect("a key or an index")),
        };
        let items: Vec<Value> = serde_json::from_value(location).expect("a JSON array");
        Self {
            kind,
            location: items.into_iter().map(segment).collect(),
            input,
            context: None,
        }
    }

    pub(crate) fn with_context(mut self, context: Value) -> Self {
        self.context = Some(context);
        self
    }

    fn to_json(&self) -> Value {
        let (kind, message) = self.kind.type_and_message();
        let location: Vec<Value> = self.location.iter().map(Segment::to_json).collect();
        let mut error = json!({
            "type": kind,
            "loc": location,
            "msg": message,
            "input": self.input,
        });
        if let Some(context) = &self.context {
            error["ctx"] = context.clone();
        }
        error
    }
}

// written straight from the errors, since building a `Value` per error first doubles the work
impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, message) = self.kind.type_and_message();
        write!(formatter, r#"{{"type":"{kind}","loc":["#)?;
        for (index, segment) in self.location.iter().enumerate() {
            if index > 0 {
                formatter.write_str(",")?;
            }
            write!(formatter, "{segment}")?;
        }
        write!(formatter, r#"],"msg":"{message}","input":{}"#, self.input)?;
        if let Some(context) = &self.context {
            write!(formatter, r#","ctx":{context}"#)?;
        }
        formatter.write_str("}")
    }
}

// keeps going after a failure so the response lists every problem, like FastAPI
pub(crate) fn collect_all<T>(
    results: impl IntoIterator<Item = Result<T, Vec<ValidationError>>>,
) -> Result<Vec<T>, Vec<ValidationError>> {
    let mut values = Vec::new();
    let mut errors = Vec::new();
    for result in results {
        match result {
            Ok(value) => values.push(value),
            Err(mut failures) => errors.append(&mut failures),
        }
    }
    if errors.is_empty() {
        Ok(values)
    } else {
        Err(errors)
    }
}

// FastAPI's RequestValidationError, whose errors are the `detail` of its 422
#[derive(Debug, Clone)]
pub struct RequestValidationError {
    errors: Vec<ValidationError>,
    // built only when an exception handler asks, since the 422 body is written without them
    values: OnceLock<Vec<Value>>,
}

impl RequestValidationError {
    fn new(errors: Vec<ValidationError>) -> Self {
        Self {
            errors,
            values: OnceLock::new(),
        }
    }

    #[must_use]
    pub fn errors(&self) -> &[Value] {
        self.values
            .get_or_init(|| self.errors.iter().map(ValidationError::to_json).collect())
    }
}

impl PartialEq for RequestValidationError {
    fn eq(&self, other: &Self) -> bool {
        self.errors == other.errors
    }
}

impl Eq for RequestValidationError {}

impl fmt::Display for RequestValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.errors.len();
        let plural = if count == 1 { "" } else { "s" };
        write!(formatter, "{count} validation error{plural} in the request")
    }
}

impl std::error::Error for RequestValidationError {}

struct Detail<'errors>(&'errors [ValidationError]);

impl fmt::Display for Detail<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(r#"{"detail":["#)?;
        for (index, error) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str(",")?;
            }
            write!(formatter, "{error}")?;
        }
        formatter.write_str("]}")
    }
}

pub(crate) fn validation_failed(errors: Vec<ValidationError>) -> Response {
    let mut response = json_response(
        StatusCode::UNPROCESSABLE_ENTITY,
        Detail(&errors).to_string(),
    );
    attach_error(&mut response, Arc::new(RequestValidationError::new(errors)));
    response
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::response::response_error;

    const KINDS: [ErrorKind; 14] = [
        ErrorKind::Missing,
        ErrorKind::StringType,
        ErrorKind::IntType,
        ErrorKind::IntParsing,
        ErrorKind::IntFromFloat,
        ErrorKind::IntParsingSize,
        ErrorKind::FloatType,
        ErrorKind::FloatParsing,
        ErrorKind::FiniteNumber,
        ErrorKind::BoolType,
        ErrorKind::BoolParsing,
        ErrorKind::ListType,
        ErrorKind::ModelAttributesType,
        ErrorKind::JsonInvalid,
    ];

    #[test]
    fn a_request_validation_error_counts_its_errors() {
        let missing = ValidationError::at(ErrorKind::Missing, json!(["body"]), json!({}));
        let one = RequestValidationError::new(vec![missing.clone()]);
        let two = RequestValidationError::new(vec![missing.clone(), missing]);

        assert_eq!(one.to_string(), "1 validation error in the request");
        assert_eq!(two.to_string(), "2 validation errors in the request");
    }

    #[test]
    fn uses_the_type_and_message_pairs_of_pydantic() {
        let pairs = KINDS.map(ErrorKind::type_and_message);

        assert_eq!(
            pairs,
            [
                ("missing", "Field required"),
                ("string_type", "Input should be a valid string"),
                ("int_type", "Input should be a valid integer"),
                (
                    "int_parsing",
                    "Input should be a valid integer, unable to parse string as an integer"
                ),
                (
                    "int_from_float",
                    "Input should be a valid integer, got a number with a fractional part"
                ),
                (
                    "int_parsing_size",
                    "Unable to parse input string as an integer, exceeded maximum size"
                ),
                ("float_type", "Input should be a valid number"),
                (
                    "float_parsing",
                    "Input should be a valid number, unable to parse string as a number"
                ),
                ("finite_number", "Input should be a finite number"),
                ("bool_type", "Input should be a valid boolean"),
                (
                    "bool_parsing",
                    "Input should be a valid boolean, unable to interpret input"
                ),
                ("list_type", "Input should be a valid list"),
                (
                    "model_attributes_type",
                    "Input should be a valid dictionary or object to extract fields from"
                ),
                ("json_invalid", "JSON decode error"),
            ]
        );
    }

    #[test]
    fn builds_locations_from_the_root_down() {
        let body = Location::root("body");
        let tags_name = Arc::from("tags");
        let tags = body.key(&tags_name);

        let error = ValidationError::new(ErrorKind::StringType, &tags.index(1), json!(5));

        assert_eq!(
            error,
            ValidationError::at(ErrorKind::StringType, json!(["body", "tags", 1]), json!(5))
        );
    }

    proptest! {
        #[test]
        fn writes_keys_the_way_serde_json_does(key in any::<String>()) {
            let segment = Segment::Key(Arc::from(key.as_str()));

            prop_assert_eq!(segment.to_string(), Value::from(key).to_string());
        }
    }

    #[test]
    fn writes_the_short_escapes_the_way_serde_json_does() {
        let key = "\"\\\u{8}\u{c}\n\r\t\u{1}/\u{e9}";

        assert_eq!(
            Segment::Key(Arc::from(key)).to_string(),
            Value::from(key).to_string()
        );
    }

    #[test]
    fn renders_errors_in_fastapi_key_order() {
        let errors = vec![
            ValidationError::at(ErrorKind::StringType, json!(["body", "tags", 1]), json!(5)),
            ValidationError::at(ErrorKind::JsonInvalid, json!(["body"]), json!({}))
                .with_context(json!({ "error": "expected value" })),
        ];

        let response = validation_failed(errors);

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(
            response.body().as_bytes().unwrap(),
            concat!(
                r#"{"detail":["#,
                r#"{"type":"string_type","loc":["body","tags",1],"msg":"Input should be a valid string","input":5},"#,
                r#"{"type":"json_invalid","loc":["body"],"msg":"JSON decode error","input":{},"ctx":{"error":"expected value"}}"#,
                r"]}"
            )
        );
    }

    #[test]
    fn types_and_messages_need_no_escaping_in_json() {
        for (kind, message) in KINDS.map(ErrorKind::type_and_message) {
            assert_eq!(Value::from(kind).to_string(), format!("\"{kind}\""));
            assert_eq!(Value::from(message).to_string(), format!("\"{message}\""));
        }
    }

    #[test]
    fn the_body_holds_the_errors_an_exception_handler_sees() {
        let errors = KINDS
            .into_iter()
            .enumerate()
            .map(|(index, kind)| {
                let error = ValidationError::at(
                    kind,
                    json!(["body", "a \"quoted\" key", index]),
                    json!({ "text": "line\nbreak \u{e9}", "list": [1.5, null, true] }),
                );
                if index % 2 == 0 {
                    error.with_context(json!({ "error": "back\\slash" }))
                } else {
                    error
                }
            })
            .collect();

        let response = validation_failed(errors);
        let body: Value = serde_json::from_slice(response.body().as_bytes().unwrap()).unwrap();
        let error = response_error(&response)
            .and_then(|error| error.downcast_ref::<RequestValidationError>())
            .unwrap();

        assert_eq!(body, json!({ "detail": error.errors() }));
    }

    #[test]
    fn equality_ignores_whether_the_errors_were_built_as_json() {
        let missing = ValidationError::at(ErrorKind::Missing, json!(["body"]), json!({}));
        let built = RequestValidationError::new(vec![missing.clone()]);
        let unbuilt = RequestValidationError::new(vec![missing]);

        assert_eq!(built.errors().len(), 1);
        assert_eq!(built, unbuilt);
    }
}
