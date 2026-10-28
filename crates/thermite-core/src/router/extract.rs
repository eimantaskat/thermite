use std::borrow::Cow;

use http::HeaderName;
use http::header::{CONTENT_TYPE, COOKIE};
use serde_json::error::Category;
use serde_json::{Value, json};

use super::check::Inputs;
use super::path::PathParameters;
use crate::arguments::Arguments;
use crate::description::{Parameter, ParameterLocation};
use crate::handler::Request;
use crate::parse::{decode_latin1, parse_cookies, parse_query};
use crate::validation::{ErrorKind, Location, ValidationError, collect_all};

pub(super) fn extract_arguments(
    inputs: &Inputs,
    path_parameters: &PathParameters,
    request: &Request,
) -> Result<Arguments, Vec<ValidationError>> {
    // like FastAPI, a body that is not JSON fails on its own, before any parameter is checked
    let body = inputs
        .body
        .as_ref()
        .map(|schema| read_body(request).map(|input| (schema, input)))
        .transpose()?;

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
    let body = body
        .map(|(schema, input)| {
            let location = Location::root("body");
            match input {
                Some(input) => schema.validate(input, &location),
                None => Err(vec![ValidationError::new(
                    ErrorKind::Missing,
                    &location,
                    Value::Null,
                )]),
            }
        })
        .transpose();
    let after_body = validate(after_body);

    match (before_body, body, after_body) {
        (Ok(mut parameters), Ok(body), Ok(after_body)) => {
            parameters.extend(after_body);
            Ok(Arguments::new(parameters, body))
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
) -> Result<(ParameterLocation, String, Value), Vec<ValidationError>> {
    let name = parameter.field().name();
    let root = Location::root(parameter.location().as_str());
    parameter
        .field()
        .validate(input, &root.key(name), None)
        .map(|value| (parameter.location(), name.to_owned(), value))
}

// a list collects every value of a repeated name, a single value takes the last one like Starlette
fn query_input(parameter: &Parameter, query: &[(Cow<'_, str>, Cow<'_, str>)]) -> Option<Value> {
    let name = parameter.field().name();
    let mut values = query
        .iter()
        .filter(|(found, _)| found == name)
        .map(|(_, value)| Value::from(value.as_ref()));
    if parameter.field().schema().is_array() {
        list(values)
    } else {
        values.next_back()
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

// FastAPI parses the body as JSON when the content type says JSON or is missing, and passes it on as text otherwise
fn read_body(request: &Request) -> Result<Option<Value>, Vec<ValidationError>> {
    let body = request.body();
    if body.is_empty() {
        return Ok(None);
    }
    if !declares_json(request) {
        return Ok(Some(Value::String(
            String::from_utf8_lossy(body).into_owned(),
        )));
    }
    serde_json::from_slice(body)
        .map(Some)
        .map_err(|error| vec![json_invalid(body, &error)])
}

fn declares_json(request: &Request) -> bool {
    // FastAPI checks `if not content_type`, so an empty header counts as missing
    let Some(content_type) = request
        .headers()
        .get(CONTENT_TYPE)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    let media_type = content_type
        .to_str()
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media_type
        .strip_prefix("application/")
        .is_some_and(|subtype| subtype == "json" || subtype.ends_with("+json"))
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
    use bytes::Bytes;
    use http::{HeaderValue, Method};

    use super::*;
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
    }

    fn extract(
        description: &RouteDescription,
        request: &Request,
    ) -> Result<Arguments, Vec<ValidationError>> {
        extract_arguments(&inputs(description), &PathParameters::default(), request)
    }

    fn json_invalid_at(position: usize, message: &str) -> Result<Arguments, Vec<ValidationError>> {
        Err(vec![
            ValidationError::at(ErrorKind::JsonInvalid, json!(["body", position]), json!({}))
                .with_context(json!({ "error": message })),
        ])
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
            object,
            Err(vec![ValidationError::at(
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

        assert_eq!(errors, json_invalid_at(9, "expected value"));
    }

    #[test]
    fn reports_a_body_cut_short_at_its_end_like_python() {
        let description = with_body(Schema::integer());

        let cut = extract(
            &description,
            &request("/items", None, r#"{"name": "anvil","#),
        );
        let blank = extract(&description, &request("/items", None, "   "));

        assert_eq!(cut, json_invalid_at(17, "EOF while parsing a value"));
        assert_eq!(blank, json_invalid_at(3, "EOF while parsing a value"));
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
            errors,
            Err(vec![
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

        let errors = extract_arguments(
            &inputs(&description),
            &path_parameters,
            &request("/items/x", None, ""),
        );

        assert_eq!(
            errors,
            Err(vec![
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
