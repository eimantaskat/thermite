use std::fmt;
use std::sync::Arc;

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
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum LocationItem<'text> {
    Key(&'text str),
    Index(usize),
}

// each level borrows its parent, so nothing is allocated until an error needs the full path
#[derive(Debug)]
pub(crate) struct Location<'parent> {
    parent: Option<&'parent Location<'parent>>,
    item: LocationItem<'parent>,
}

impl<'parent> Location<'parent> {
    pub(crate) fn root(source: &'parent str) -> Self {
        Self {
            parent: None,
            item: LocationItem::Key(source),
        }
    }

    pub(crate) fn key<'child>(&'child self, key: &'child str) -> Location<'child> {
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

    fn to_json(&self) -> Value {
        let mut items = Vec::new();
        let mut current = Some(self);
        while let Some(location) = current {
            items.push(match location.item {
                LocationItem::Key(key) => Value::from(key),
                LocationItem::Index(index) => Value::from(index),
            });
            current = location.parent;
        }
        items.reverse();
        Value::Array(items)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidationError {
    kind: ErrorKind,
    location: Value,
    input: Value,
    context: Option<Value>,
}

impl ValidationError {
    pub(crate) fn new(kind: ErrorKind, location: &Location<'_>, input: Value) -> Self {
        Self::at(kind, location.to_json(), input)
    }

    // takes `loc` already as JSON, so tests can write it the way it appears in the response
    pub(crate) fn at(kind: ErrorKind, location: Value, input: Value) -> Self {
        Self {
            kind,
            location,
            input,
            context: None,
        }
    }

    pub(crate) fn with_context(mut self, context: Value) -> Self {
        self.context = Some(context);
        self
    }

    fn into_json(self) -> Value {
        let (kind, message) = self.kind.type_and_message();
        let mut error = json!({
            "type": kind,
            "loc": self.location,
            "msg": message,
            "input": self.input,
        });
        if let Some(context) = self.context {
            error["ctx"] = context;
        }
        error
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestValidationError {
    errors: Vec<Value>,
}

impl RequestValidationError {
    #[must_use]
    pub fn errors(&self) -> &[Value] {
        &self.errors
    }
}

impl fmt::Display for RequestValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.errors.len();
        let plural = if count == 1 { "" } else { "s" };
        write!(formatter, "{count} validation error{plural} in the request")
    }
}

impl std::error::Error for RequestValidationError {}

pub(crate) fn validation_failed(errors: Vec<ValidationError>) -> Response {
    let errors: Vec<Value> = errors.into_iter().map(ValidationError::into_json).collect();
    let mut response = json_response(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "detail": errors }).to_string(),
    );
    attach_error(&mut response, Arc::new(RequestValidationError { errors }));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_validation_error_counts_its_errors() {
        let one = RequestValidationError {
            errors: vec![json!({"type": "missing"})],
        };
        let two = RequestValidationError {
            errors: vec![json!({"type": "missing"}), json!({"type": "int_parsing"})],
        };

        assert_eq!(one.to_string(), "1 validation error in the request");
        assert_eq!(two.to_string(), "2 validation errors in the request");
    }

    #[test]
    fn uses_the_type_and_message_pairs_of_pydantic() {
        let pairs = [
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
        ]
        .map(ErrorKind::type_and_message);

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
        let tags = body.key("tags");

        let error = ValidationError::new(ErrorKind::StringType, &tags.index(1), json!(5));

        assert_eq!(
            error,
            ValidationError::at(ErrorKind::StringType, json!(["body", "tags", 1]), json!(5))
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
            response.body(),
            concat!(
                r#"{"detail":["#,
                r#"{"type":"string_type","loc":["body","tags",1],"msg":"Input should be a valid string","input":5},"#,
                r#"{"type":"json_invalid","loc":["body"],"msg":"JSON decode error","input":{},"ctx":{"error":"expected value"}}"#,
                r"]}"
            )
        );
    }
}
