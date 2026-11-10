use std::any::Any;
use std::borrow::Cow;
use std::sync::Arc;

use http::HeaderName;
use http::header::{CONTENT_TYPE, COOKIE};
use serde_json::error::Category;
use serde_json::{Value, json};

use super::check::Inputs;
use super::path::PathParameters;
use crate::arguments::Arguments;
use crate::description::{BodyEncoding, Parameter, ParameterLocation};
use crate::handler::Request;
use crate::parse::{decode_latin1, parse_cookies, parse_query};
use crate::schema::{Field, Schema};
use crate::validation::{ErrorKind, Location, ValidationError, collect_all};

// what the body gave, read on its own so that a large one can be read away from the other requests
#[derive(Default)]
pub(super) struct ReadBody {
    decoded: Option<Box<dyn Any + Send>>,
    validated: Option<Result<Value, Vec<ValidationError>>>,
}

// what the request's content type says its body is
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Content {
    Json,
    Form,
    Other,
}

pub(super) fn read_body(
    inputs: &Inputs,
    body: &[u8],
    content: Content,
) -> Result<ReadBody, Vec<ValidationError>> {
    let Some(schema) = &inputs.body else {
        return Ok(ReadBody::default());
    };
    let location = Location::root("body");
    let is_json = content == Content::Json;
    let decoded = match &inputs.encoding {
        BodyEncoding::Form => {
            let validated = schema.validate(form_input(schema, body, content), &location);
            return Ok(ReadBody {
                decoded: None,
                validated: Some(validated),
            });
        }
        BodyEncoding::Json(Some(decoder)) if !body.is_empty() && is_json => decoder.decode(body),
        BodyEncoding::Json(_) => None,
    };
    // a decoded body is valid, so only the parameters are left to report
    if decoded.is_some() {
        return Ok(ReadBody {
            decoded,
            validated: None,
        });
    }
    // like FastAPI, a body that is not JSON fails on its own, before any parameter is checked
    let validated = match parse_body(body, is_json)? {
        Some(input) => schema.validate(input, &location),
        None => Err(vec![ValidationError::new(
            ErrorKind::Missing,
            &location,
            Value::Null,
        )]),
    };
    Ok(ReadBody {
        decoded: None,
        validated: Some(validated),
    })
}

pub(super) fn extract_arguments(
    inputs: &Inputs,
    path_parameters: &PathParameters<'_, '_>,
    request: &Request,
    body: ReadBody,
) -> Result<Arguments, Vec<ValidationError>> {
    if inputs.parameters.is_empty() && inputs.body.is_none() {
        return Ok(Arguments::default());
    }
    let query = parse_query(request.uri().query().unwrap_or(""));
    let reads_cookies = inputs
        .parameters
        .iter()
        .any(|parameter| parameter.location() == ParameterLocation::Cookie);
    // like Starlette, only the first Cookie header is read
    let cookie_header = request
        .headers()
        .get(COOKIE)
        .filter(|_| reads_cookies)
        .map(|value| decode_latin1(value.as_bytes()));
    let cookies = cookie_header
        .as_deref()
        .map(parse_cookies)
        .unwrap_or_default();
    let validate = |parameters: &[Parameter]| {
        collect_all(parameters.iter().map(|parameter| {
            let name = parameter.field().name();
            let input = match parameter.location() {
                ParameterLocation::Path => path_parameters.get(name).map(Value::from),
                ParameterLocation::Query => query_input(parameter, &query),
                ParameterLocation::Header => parameter
                    .header_name()
                    .and_then(|header_name| header_input(parameter, header_name, request)),
                ParameterLocation::Cookie => cookies
                    .iter()
                    .rev()
                    .find(|(found, _)| *found == name)
                    .map(|(_, value)| Value::from(value.as_ref())),
            };
            validate_parameter(parameter, input)
        }))
    };
    let (before_body, after_body) = inputs.parameters.split_at(inputs.body_position);
    let before_body = validate(before_body);
    let validated = body.validated.transpose();
    let after_body = validate(after_body);

    match (before_body, validated, after_body) {
        (Ok(mut parameters), Ok(validated), Ok(after_body)) => {
            parameters.extend(after_body);
            Ok(Arguments::new(parameters, validated, body.decoded))
        }
        (before_body, body, after_body) => Err(before_body
            .err()
            .into_iter()
            .chain(body.err())
            .chain(after_body.err())
            .flatten()
            .collect()),
    }
}

fn validate_parameter(
    parameter: &Parameter,
    input: Option<Value>,
) -> Result<(ParameterLocation, Arc<str>, Value), Vec<ValidationError>> {
    let field = parameter.field();
    let root = Location::root(parameter.location().as_str());
    field
        .validate(input, &root.key(field.shared_name()), None)
        .map(|value| (parameter.location(), Arc::clone(field.shared_name()), value))
}

fn query_input(parameter: &Parameter, query: &[(Cow<'_, str>, Cow<'_, str>)]) -> Option<Value> {
    named_values(query, parameter.field(), |_| true)
}

// a list collects every value of a repeated name, a single value takes the last one like Starlette, if `keep` keeps it
fn named_values(
    pairs: &[(Cow<'_, str>, Cow<'_, str>)],
    field: &Field,
    keep: impl Fn(&str) -> bool,
) -> Option<Value> {
    let mut values = pairs
        .iter()
        .filter(|(found, _)| found == field.name())
        .map(|(_, value)| value.as_ref());
    if field.schema().is_array() {
        list(values.map(Value::from))
    } else {
        values
            .next_back()
            .filter(|value| keep(value))
            .map(Value::from)
    }
}

// a list collects every header of the name, a single value takes the first one like Starlette
fn header_input(
    parameter: &Parameter,
    header_name: &HeaderName,
    request: &Request,
) -> Option<Value> {
    let mut values = request
        .headers()
        .get_all(header_name)
        .iter()
        .map(|value| Value::from(decode_latin1(value.as_bytes())));
    if parameter.field().schema().is_array() {
        list(values)
    } else {
        values.next()
    }
}

fn list(values: impl Iterator<Item = Value>) -> Option<Value> {
    let values: Vec<Value> = values.collect();
    (!values.is_empty()).then_some(Value::Array(values))
}

// the declared fields as an object of strings; like Starlette, a body of another content type is an empty form
fn form_input(schema: &Schema, body: &[u8], content: Content) -> Value {
    let text = match content {
        Content::Form => decode_latin1(body),
        Content::Json | Content::Other => Cow::Borrowed(""),
    };
    let pairs = parse_query(&text);
    let fields = schema.fields().unwrap_or_default();
    // like FastAPI, an empty value is missing, though a list keeps its empty items
    let members = fields.iter().filter_map(|field| {
        named_values(&pairs, field, |value| !value.is_empty())
            .map(|value| (field.name().to_owned(), value))
    });
    Value::Object(members.collect())
}

// FastAPI parses the body as JSON when the content type says JSON or is missing, and passes it on as text otherwise
fn parse_body(body: &[u8], is_json: bool) -> Result<Option<Value>, Vec<ValidationError>> {
    if body.is_empty() {
        return Ok(None);
    }
    if !is_json {
        return Ok(Some(Value::String(
            String::from_utf8_lossy(body).into_owned(),
        )));
    }
    serde_json::from_slice(body)
        .map(Some)
        .map_err(|error| vec![json_invalid(body, &error)])
}

pub(super) fn content(request: &Request) -> Content {
    // FastAPI checks `if not content_type`, so an empty header counts as missing
    let Some(content_type) = request
        .headers()
        .get(CONTENT_TYPE)
        .filter(|value| !value.is_empty())
    else {
        return Content::Json;
    };
    let media_type = content_type
        .to_str()
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match media_type.strip_prefix("application/") {
        Some("x-www-form-urlencoded") => Content::Form,
        Some(subtype) if subtype == "json" || subtype.ends_with("+json") => Content::Json,
        _ => Content::Other,
    }
}

fn json_invalid(body: &[u8], error: &serde_json::Error) -> ValidationError {
    let (line, column) = (error.line(), error.column());
    let suffix = format!(" at line {line} column {column}");
    let message = error.to_string();
    let message = message.strip_suffix(&suffix).unwrap_or(&message);
    // at the end of the input serde_json points at the last byte, Python just past it
    let position = if error.classify() == Category::Eof {
        body.len()
    } else {
        byte_offset(body, line, column)
    };
    let root = Location::root("body");
    ValidationError::new(ErrorKind::JsonInvalid, &root.index(position), json!({}))
        .with_context(json!({ "error": message }))
}

// serde_json reports a line and a 1-based column, FastAPI an offset from the start
fn byte_offset(body: &[u8], line: usize, column: usize) -> usize {
    let line_start: usize = body
        .split(|&byte| byte == b'\n')
        .take(line.saturating_sub(1))
        .map(|text| text.len() + 1)
        .sum();
    line_start + column.saturating_sub(1)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::any::Any;

    use bytes::Bytes;
    use http::{HeaderValue, Method};

    use super::*;
    use crate::decoder::BodyDecoder;
    use crate::description::RouteDescription;
    use crate::router::check::check_description;
    use crate::router::path::PathPattern;
    use crate::schema::{Field, Schema};

    fn with_headers(uri: &str, headers: &[(&'static str, &'static [u8])]) -> Request {
        let mut request = request(uri, None, "");
        for (name, value) in headers {
            request
                .headers_mut()
                .append(*name, HeaderValue::from_bytes(value).unwrap());
        }
        request
    }

    fn request(uri: &str, content_type: Option<&'static str>, body: &'static str) -> Request {
        let mut request = Request::new(Bytes::from_static(body.as_bytes()));
        *request.uri_mut() = uri.parse().unwrap();
        if let Some(content_type) = content_type {
            request
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
        }
        request
    }

    fn with_body(schema: Schema) -> RouteDescription {
        RouteDescription::new(Method::POST, "/items").body(schema)
    }

    fn inputs(description: &RouteDescription) -> Inputs {
        check_description(
            description.clone(),
            &PathPattern::parse(description.path()).unwrap(),
        )
        .unwrap()
        .inputs
    }

    fn extract(
        description: &RouteDescription,
        request: &Request,
    ) -> Result<Arguments, Vec<ValidationError>> {
        extract_at(description, &PathParameters::default(), request)
    }

    fn extract_at(
        description: &RouteDescription,
        path_parameters: &PathParameters<'_, '_>,
        request: &Request,
    ) -> Result<Arguments, Vec<ValidationError>> {
        let inputs = inputs(description);
        let body = read_body(&inputs, request.body(), content(request))?;
        extract_arguments(&inputs, path_parameters, request, body)
    }

    // takes a JSON integer as it is, and gives up on anything validation would coerce or reject
    struct Integer;

    impl BodyDecoder for Integer {
        fn decode(&self, body: &[u8]) -> Option<Box<dyn Any + Send>> {
            let integer: i64 = serde_json::from_slice(body).ok()?;
            Some(Box::new(integer))
        }
    }

    fn with_decoded_body() -> RouteDescription {
        RouteDescription::new(Method::POST, "/items").body_with_decoder(Schema::integer(), Integer)
    }

    fn decoded(arguments: &Arguments) -> Option<i64> {
        let decoded = arguments.take_decoded_body()?;
        decoded.downcast::<i64>().ok().map(|integer| *integer)
    }

    #[test]
    fn a_decoded_body_is_not_also_validated() {
        let arguments = extract(&with_decoded_body(), &request("/items", None, "42")).unwrap();

        assert_eq!(arguments.body(), None);
        assert_eq!(decoded(&arguments), Some(42));
    }

    #[test]
    fn a_body_the_decoder_gives_up_on_is_validated() {
        let coerced = extract(&with_decoded_body(), &request("/items", None, "\"7\"")).unwrap();
        let invalid = extract(&with_decoded_body(), &request("/items", None, "\"x\""));
        let broken = extract(&with_decoded_body(), &request("/items", None, "4 2"));

        assert_eq!(coerced.body(), Some(&json!(7)));
        assert_eq!(decoded(&coerced), None);
        assert_eq!(
            invalid.err(),
            Some(vec![ValidationError::at(
                ErrorKind::IntParsing,
                json!(["body"]),
                json!("x")
            )])
        );
        assert_eq!(
            broken.err(),
            Some(json_invalid_at(2, "trailing characters"))
        );
    }

    #[test]
    fn only_a_json_body_is_decoded() {
        let text = extract(
            &with_decoded_body(),
            &request("/items", Some("text/plain"), "42"),
        )
        .unwrap();
        let empty = extract(&with_decoded_body(), &request("/items", None, ""));

        assert_eq!(text.body(), Some(&json!(42)));
        assert_eq!(decoded(&text), None);
        assert_eq!(
            empty.err(),
            Some(vec![ValidationError::at(
                ErrorKind::Missing,
                json!(["body"]),
                json!(null)
            )])
        );
    }

    #[test]
    fn a_decoded_body_leaves_only_the_parameter_errors() {
        let description =
            with_decoded_body().parameter(Parameter::query("limit", Schema::integer()));

        let errors = extract(&description, &request("/items?limit=x", None, "42"));

        assert_eq!(
            errors.err(),
            Some(vec![ValidationError::at(
                ErrorKind::IntParsing,
                json!(["query", "limit"]),
                json!("x")
            )])
        );
    }

    #[test]
    fn a_body_read_in_two_places_is_validated_instead_of_decoded() {
        let description = with_decoded_body()
            .dependency(|dependency| dependency.body_with_decoder(Schema::integer(), Integer));

        let arguments = extract(&description, &request("/items", None, "42")).unwrap();

        assert_eq!(arguments.body(), Some(&json!(42)));
        assert_eq!(decoded(&arguments), None);
    }

    fn json_invalid_at(position: usize, message: &str) -> Vec<ValidationError> {
        vec![
            ValidationError::at(ErrorKind::JsonInvalid, json!(["body", position]), json!({}))
                .with_context(json!({ "error": message })),
        ]
    }

    #[test]
    fn parses_json_bodies_by_content_type_or_without_one() {
        let description = with_body(Schema::integer());
        for content_type in [
            None,
            Some(""),
            Some("application/json"),
            Some("Application/JSON; charset=utf-8"),
            Some("application/merge-patch+json"),
        ] {
            let arguments = extract(&description, &request("/items", content_type, "5")).unwrap();

            assert_eq!(arguments.body(), Some(&json!(5)), "{content_type:?}");
        }
    }

    fn with_form(fields: impl IntoIterator<Item = Field>) -> RouteDescription {
        RouteDescription::new(Method::POST, "/login").form_body(Schema::object(fields))
    }

    const FORM: Option<&str> = Some("application/x-www-form-urlencoded");

    #[test]
    fn reads_form_fields_coerced_from_their_strings() {
        let description = with_form([
            Field::new("name", Schema::string()),
            Field::new("count", Schema::integer()),
            Field::new("tags", Schema::array(Schema::string())),
        ]);
        let body = "tags=a&name=big+anvil%21&count=3&tags=&tags=b&other=1";

        let arguments = extract(&description, &request("/login", FORM, body)).unwrap();

        assert_eq!(
            arguments.body(),
            Some(&json!({"name": "big anvil!", "count": 3, "tags": ["a", "", "b"]}))
        );
    }

    #[test]
    fn a_repeated_form_field_takes_the_last_value_like_starlette() {
        let description = with_form([Field::new("name", Schema::string())]);
        let request = request(
            "/login",
            Some("Application/X-WWW-Form-Urlencoded; x=y"),
            "name=a&name=b",
        );

        let arguments = extract(&description, &request).unwrap();

        assert_eq!(arguments.body(), Some(&json!({"name": "b"})));
    }

    #[test]
    fn an_empty_form_value_is_missing_like_fastapi() {
        let required = with_form([
            Field::new("name", Schema::string()),
            Field::new("count", Schema::integer()),
        ]);
        let defaulted = with_form([
            Field::new("name", Schema::string()).with_default("guest"),
            Field::new("tags", Schema::array(Schema::string())).optional(),
        ]);

        let missing = extract(&required, &request("/login", FORM, "name=&count=3"));
        let default = extract(&defaulted, &request("/login", FORM, "name=")).unwrap();

        assert_eq!(
            missing.err(),
            Some(vec![ValidationError::at(
                ErrorKind::Missing,
                json!(["body", "name"]),
                json!({"count": "3"})
            )])
        );
        assert_eq!(
            default.body(),
            Some(&json!({"name": "guest", "tags": null}))
        );
    }

    #[test]
    fn a_body_of_another_content_type_is_an_empty_form_like_starlette() {
        let description = with_form([Field::new("name", Schema::string())]);

        for content_type in [None, Some("application/json"), Some("text/plain")] {
            let read = extract(&description, &request("/login", content_type, "name=a"));

            assert_eq!(
                read.err(),
                Some(vec![ValidationError::at(
                    ErrorKind::Missing,
                    json!(["body", "name"]),
                    json!({})
                )]),
                "{content_type:?}"
            );
        }
    }

    #[test]
    fn a_json_body_sent_as_a_form_is_text() {
        let text = extract(
            &with_body(Schema::string()),
            &request("/items", FORM, "name=a"),
        )
        .unwrap();

        assert_eq!(text.body(), Some(&json!("name=a")));
    }

    #[test]
    fn passes_bodies_of_other_content_types_on_as_text() {
        let text = extract(
            &with_body(Schema::string()),
            &request("/items", Some("text/plain"), "{not json"),
        )
        .unwrap();
        let object = extract(
            &with_body(Schema::object([Field::new("a", Schema::string())])),
            &request("/items", Some("text/plain"), "{}"),
        );

        assert_eq!(text.body(), Some(&json!("{not json")));
        assert_eq!(
            object.err(),
            Some(vec![ValidationError::at(
                ErrorKind::ModelAttributesType,
                json!(["body"]),
                json!("{}")
            )])
        );
    }

    #[test]
    fn reports_where_invalid_json_stops_parsing() {
        let description =
            with_body(Schema::integer()).parameter(Parameter::query("limit", Schema::integer()));

        let errors = extract(&description, &request("/items", None, "{\n  \"a\": }"));

        assert_eq!(errors.err(), Some(json_invalid_at(9, "expected value")));
    }

    #[test]
    fn reports_a_body_cut_short_at_its_end_like_python() {
        let description = with_body(Schema::integer());

        let cut = extract(
            &description,
            &request("/items", None, r#"{"name": "anvil","#),
        );
        let blank = extract(&description, &request("/items", None, "   "));

        assert_eq!(
            cut.err(),
            Some(json_invalid_at(17, "EOF while parsing a value"))
        );
        assert_eq!(
            blank.err(),
            Some(json_invalid_at(3, "EOF while parsing a value"))
        );
    }

    #[test]
    fn computes_offsets_across_lines() {
        assert_eq!(byte_offset(b"abc", 1, 2), 1);
        assert_eq!(byte_offset(b"ab\ncd\nef", 3, 1), 6);
    }

    #[test]
    fn collects_repeated_query_values_for_a_list_and_takes_the_last_otherwise() {
        let description = RouteDescription::new(Method::GET, "/items")
            .parameter(Parameter::query("tag", Schema::array(Schema::string())))
            .parameter(Parameter::query("limit", Schema::integer()));

        let arguments = extract(
            &description,
            &request("/items?tag=a&limit=1&tag=b&limit=2", None, ""),
        )
        .unwrap();

        assert_eq!(arguments.query("tag"), Some(&json!(["a", "b"])));
        assert_eq!(arguments.query("limit"), Some(&json!(2)));
    }

    #[test]
    fn matches_header_names_in_any_case_and_decodes_values_as_latin_1() {
        let description = RouteDescription::new(Method::GET, "/items")
            .parameter(Parameter::header("X-Token", Schema::string()))
            .parameter(Parameter::header("x-tag", Schema::array(Schema::string())))
            .parameter(Parameter::header("x-missing", Schema::array(Schema::string())).optional());

        let arguments = extract(
            &description,
            &with_headers(
                "/items",
                &[
                    ("x-token", b"caf\xe9"),
                    ("x-token", b"second"),
                    ("x-tag", b"a, b"),
                    ("x-tag", b"c"),
                ],
            ),
        )
        .unwrap();

        assert_eq!(arguments.header("X-Token"), Some(&json!("caf\u{e9}")));
        assert_eq!(arguments.header("x-tag"), Some(&json!(["a, b", "c"])));
        assert_eq!(arguments.header("x-missing"), Some(&json!(null)));
    }

    #[test]
    fn reads_cookies_from_the_first_cookie_header_and_takes_the_last_of_a_name() {
        let description = RouteDescription::new(Method::GET, "/items")
            .parameter(Parameter::cookie("session", Schema::string()))
            .parameter(Parameter::cookie("theme", Schema::string()).with_default("light"));

        let arguments = extract(
            &description,
            &with_headers(
                "/items",
                &[
                    ("cookie", b"session=a; session=\"b c\""),
                    ("cookie", b"theme=dark"),
                ],
            ),
        )
        .unwrap();

        assert_eq!(arguments.cookie("session"), Some(&json!("b c")));
        assert_eq!(arguments.cookie("theme"), Some(&json!("light")));
    }

    #[test]
    fn reports_the_body_errors_of_a_dependency_in_its_place() {
        let description = RouteDescription::new(Method::POST, "/items")
            .parameter(Parameter::query("limit", Schema::integer()))
            .dependency(|dependency| {
                dependency
                    .parameter(Parameter::header("x-key", Schema::string()))
                    .body(Schema::integer())
            });

        let errors = extract(&description, &request("/items?limit=x", None, "\"text\""));

        assert_eq!(
            errors.err(),
            Some(vec![
                ValidationError::at(ErrorKind::Missing, json!(["header", "x-key"]), json!(null)),
                ValidationError::at(ErrorKind::IntParsing, json!(["body"]), json!("text")),
                ValidationError::at(ErrorKind::IntParsing, json!(["query", "limit"]), json!("x")),
            ])
        );
    }

    #[test]
    fn reports_errors_by_location_then_body_errors() {
        let description = RouteDescription::new(Method::POST, "/items/{id}")
            .parameter(Parameter::cookie("session", Schema::string()))
            .parameter(Parameter::header("x-count", Schema::integer()))
            .parameter(Parameter::query("tag", Schema::array(Schema::string())))
            .parameter(Parameter::path("id", Schema::integer()))
            .body(Schema::string());
        let path_parameters = PathParameters::from_pairs([("id", "x")]);

        let errors = extract_at(
            &description,
            &path_parameters,
            &request("/items/x", None, ""),
        );

        assert_eq!(
            errors.err(),
            Some(vec![
                ValidationError::at(ErrorKind::IntParsing, json!(["path", "id"]), json!("x")),
                ValidationError::at(ErrorKind::Missing, json!(["query", "tag"]), json!(null)),
                ValidationError::at(
                    ErrorKind::Missing,
                    json!(["header", "x-count"]),
                    json!(null)
                ),
                ValidationError::at(
                    ErrorKind::Missing,
                    json!(["cookie", "session"]),
                    json!(null)
                ),
                ValidationError::at(ErrorKind::Missing, json!(["body"]), json!(null)),
            ])
        );
    }
}
