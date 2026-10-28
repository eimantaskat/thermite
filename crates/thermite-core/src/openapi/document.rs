use serde_json::{Value, json};

use super::OpenApi;
use crate::description::Parameter;
use crate::router::{Operation, Router};
use crate::schema::{Components, JsonSchema, NameConflict, entries, title};

const VALIDATION_ERROR: &str = "ValidationError";
const HTTP_VALIDATION_ERROR: &str = "HTTPValidationError";

pub(super) fn build(openapi: &OpenApi, router: &Router) -> Result<Value, NameConflict> {
    let operations = router.operations();
    let mut components = Components::default();
    let mut paths = Value::Object(JsonSchema::new());
    let mut validates = false;
    for operation in &operations {
        validates |= reads_input(operation);
        paths[operation.path][operation.method.as_str().to_lowercase()] =
            Value::Object(describe(operation, &mut components)?);
    }
    if validates {
        components.add_built_in(HTTP_VALIDATION_ERROR, http_validation_error())?;
        components.add_built_in(VALIDATION_ERROR, validation_error())?;
    }
    let mut document = entries([
        ("openapi", "3.1.0".into()),
        (
            "info",
            json!({ "title": openapi.title, "version": openapi.version }),
        ),
        ("paths", paths),
    ]);
    let schemas = components.into_json();
    if !schemas.is_empty() {
        document.insert("components".to_owned(), json!({ "schemas": schemas }));
    }
    Ok(Value::Object(document))
}

fn reads_input(operation: &Operation<'_>) -> bool {
    !operation.parameters.is_empty() || operation.body.is_some()
}

// in FastAPI's key order
fn describe<'router>(
    operation: &Operation<'router>,
    components: &mut Components<'router>,
) -> Result<JsonSchema, NameConflict> {
    let mut described = JsonSchema::new();
    if let Some(name) = operation.name {
        described.insert("summary".to_owned(), title(name).into());
    }
    described.insert("operationId".to_owned(), operation_id(operation).into());
    let mut parameters: Vec<&Parameter> = operation.parameters.iter().collect();
    // stable, so parameters of one location keep the order the route reads them in
    parameters.sort_by_key(|parameter| parameter.location().rank());
    let parameters = parameters
        .into_iter()
        .map(|parameter| describe_parameter(parameter, components))
        .collect::<Result<Vec<_>, _>>()?;
    if !parameters.is_empty() {
        described.insert("parameters".to_owned(), Value::Array(parameters));
    }
    if let Some(body) = operation.body {
        described.insert(
            "requestBody".to_owned(),
            json!({
                "content": { "application/json": { "schema": body.json_schema(components)? } },
                "required": true
            }),
        );
    }
    let mut responses = entries([(
        "200",
        json!({
            "description": "Successful Response",
            "content": { "application/json": { "schema": {} } }
        }),
    )]);
    if reads_input(operation) {
        responses.insert(
            "422".to_owned(),
            json!({
                "description": "Validation Error",
                "content": { "application/json": { "schema": reference(HTTP_VALIDATION_ERROR) } }
            }),
        );
    }
    described.insert("responses".to_owned(), Value::Object(responses));
    Ok(described)
}

fn describe_parameter<'router>(
    parameter: &'router Parameter,
    components: &mut Components<'router>,
) -> Result<Value, NameConflict> {
    let field = parameter.field();
    Ok(json!({
        "name": field.name(),
        "in": parameter.location().as_str(),
        "required": field.is_required(),
        "schema": field.json_schema(components)?
    }))
}

// FastAPI's generate_unique_id; a route without a name, such as a closure, goes by path and method
fn operation_id(operation: &Operation<'_>) -> String {
    let words = |text: &str| -> String {
        text.chars()
            .map(|character| {
                if character.is_alphanumeric() || character == '_' {
                    character
                } else {
                    '_'
                }
            })
            .collect()
    };
    let path = words(operation.path);
    let base = match operation.name {
        Some(name) => words(name) + &path,
        None => path.trim_start_matches('_').to_owned(),
    };
    let method = operation.method.as_str().to_lowercase();
    if base.is_empty() {
        method
    } else {
        format!("{base}_{method}")
    }
}

fn reference(name: &str) -> Value {
    json!({ "$ref": format!("#/components/schemas/{name}") })
}

// the shape of FastAPI's 422 body, as the core writes it
fn http_validation_error() -> JsonSchema {
    entries([
        (
            "properties",
            json!({
                "detail": {
                    "items": reference(VALIDATION_ERROR),
                    "title": "Detail",
                    "type": "array"
                }
            }),
        ),
        ("title", HTTP_VALIDATION_ERROR.into()),
        ("type", "object".into()),
    ])
}

fn validation_error() -> JsonSchema {
    entries([
        (
            "properties",
            json!({
                "loc": {
                    "items": { "anyOf": [{ "type": "string" }, { "type": "integer" }] },
                    "title": "Location",
                    "type": "array"
                },
                "msg": { "title": "Message", "type": "string" },
                "type": { "title": "Error Type", "type": "string" },
                "input": { "title": "Input" },
                "ctx": { "title": "Context", "type": "object" }
            }),
        ),
        ("required", json!(["loc", "msg", "type"])),
        ("title", VALIDATION_ERROR.into()),
        ("type", "object".into()),
    ])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use http::Method;

    use super::*;

    fn operation(name: Option<&'static str>, path: &'static str) -> Operation<'static> {
        Operation {
            method: &Method::GET,
            path,
            name,
            parameters: &[],
            body: None,
        }
    }

    #[test]
    fn names_operations_the_way_fastapi_does() {
        assert_eq!(
            operation_id(&operation(Some("read_item"), "/items/{item_id}")),
            "read_item_items__item_id__get"
        );
        assert_eq!(operation_id(&operation(Some("root"), "/")), "root__get");
    }

    #[test]
    fn names_an_operation_without_a_name_by_path_and_method() {
        assert_eq!(
            operation_id(&operation(None, "/items/{item_id}")),
            "items__item_id__get"
        );
        assert_eq!(operation_id(&operation(None, "/")), "get");
    }

    #[test]
    fn an_empty_router_has_paths_but_no_components() {
        let document = build(&OpenApi::new(), &Router::new()).unwrap();

        assert_eq!(
            document,
            json!({
                "openapi": "3.1.0",
                "info": { "title": "Thermite", "version": "0.1.0" },
                "paths": {}
            })
        );
    }
}
