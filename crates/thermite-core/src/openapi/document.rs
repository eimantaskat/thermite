use http::StatusCode;
use serde_json::{Value, json};

use super::{OpenApi, OpenApiError};
use crate::description::Parameter;
use crate::router::{Operation, Router};
use crate::schema::{Components, JsonSchema, NameConflict, entries, title};

const VALIDATION_ERROR: &str = "ValidationError";
const HTTP_VALIDATION_ERROR: &str = "HTTPValidationError";

pub(super) fn build(openapi: &OpenApi, router: &Router) -> Result<Value, OpenApiError> {
    let operations = router.operations();
    let mut components = Components::default();
    if operations.iter().any(reads_input) {
        components.add_built_in(HTTP_VALIDATION_ERROR, http_validation_error());
        components.add_built_in(VALIDATION_ERROR, validation_error());
    }
    let mut paths = Value::Object(JsonSchema::new());
    for operation in &operations {
        let described = describe(operation, &mut components).map_err(|NameConflict(name)| {
            OpenApiError::NameConflict {
                name,
                method: operation.method.clone(),
                path: operation.path.to_owned(),
            }
        })?;
        paths[operation.path][operation.method.as_str().to_lowercase()] = Value::Object(described);
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
    if !operation.tags.is_empty() {
        described.insert("tags".to_owned(), operation.tags.into());
    }
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
    described.insert(
        "responses".to_owned(),
        describe_responses(operation, components)?,
    );
    Ok(described)
}

// the responses of one status, in the order they were first declared
struct Documented<'router> {
    status: StatusCode,
    descriptions: Vec<&'router str>,
    schemas: Vec<Value>,
}

// like FastAPI, a route that declares no success still gets the default one, first
fn describe_responses<'router>(
    operation: &Operation<'router>,
    components: &mut Components<'router>,
) -> Result<Value, NameConflict> {
    let mut documented = Vec::new();
    if !operation
        .responses
        .iter()
        .any(|response| response.status().as_u16() < 400)
    {
        documented.push(Documented {
            status: StatusCode::OK,
            descriptions: vec!["Successful Response"],
            schemas: vec![json!({})],
        });
    }
    for response in operation.responses {
        let schema = response
            .schema()
            .map(|schema| schema.json_schema(components).map(Value::Object))
            .transpose()?;
        merge(
            &mut documented,
            response.status(),
            response.description(),
            schema,
        );
    }
    if reads_input(operation) {
        merge(
            &mut documented,
            StatusCode::UNPROCESSABLE_ENTITY,
            "Validation Error",
            Some(reference(HTTP_VALIDATION_ERROR)),
        );
    }
    Ok(documented
        .into_iter()
        .map(|documented| {
            (
                documented.status.as_str().to_owned(),
                documented.into_json(),
            )
        })
        .collect::<JsonSchema>()
        .into())
}

// unlike FastAPI, where a declared 422 replaces its own, every description and body of a status is listed
fn merge<'router>(
    documented: &mut Vec<Documented<'router>>,
    status: StatusCode,
    description: &'router str,
    schema: Option<Value>,
) {
    let known = documented.iter().position(|known| known.status == status);
    let index = known.unwrap_or_else(|| {
        documented.push(Documented {
            status,
            descriptions: Vec::new(),
            schemas: Vec::new(),
        });
        documented.len() - 1
    });
    let known = &mut documented[index];
    if !known.descriptions.contains(&description) {
        known.descriptions.push(description);
    }
    if let Some(schema) = schema
        && !known.schemas.contains(&schema)
    {
        known.schemas.push(schema);
    }
}

impl Documented<'_> {
    fn into_json(mut self) -> Value {
        let mut response = entries([("description", self.descriptions.join(" or ").into())]);
        let schema = match self.schemas.len() {
            0 => None,
            1 => self.schemas.pop(),
            _ => Some(json!({ "anyOf": self.schemas })),
        };
        if let Some(schema) = schema {
            response.insert(
                "content".to_owned(),
                json!({ "application/json": { "schema": schema } }),
            );
        }
        Value::Object(response)
    }
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
    use bytes::Bytes;
    use http::Method;

    use super::*;
    use crate::arguments::Arguments;
    use crate::description::{ResponseDescription, RouteDescription};
    use crate::handler::{Request, Response};
    use crate::schema::{Field, Schema};

    fn operation(name: Option<&'static str>, path: &'static str) -> Operation<'static> {
        Operation {
            method: &Method::GET,
            path,
            name,
            tags: &[],
            parameters: &[],
            body: None,
            responses: &[],
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

    fn items() -> RouteDescription {
        RouteDescription::new(Method::POST, "/items")
    }

    fn document(description: RouteDescription) -> Result<Value, OpenApiError> {
        let mut router = Router::new();
        router
            .route(description, |_: Request, _: Arguments| async {
                Response::new(Bytes::new())
            })
            .unwrap();
        build(&OpenApi::new(), &router)
    }

    fn responses(description: RouteDescription) -> Value {
        document(description).unwrap()["paths"]["/items"]["post"]["responses"].take()
    }

    // JSON objects compare equal in any key order, while the docs show them in this one
    fn statuses(responses: &Value) -> Vec<&str> {
        responses
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect()
    }

    fn json_content(schema: Value) -> Value {
        let media_type = entries([("schema", schema)]);
        Value::Object(entries([("application/json", Value::Object(media_type))]))
    }

    #[test]
    fn a_route_without_a_declared_success_gets_the_default_one_first() {
        let conflict = Schema::model("Conflict", [Field::new("detail", Schema::string())]);

        let responses = responses(
            items()
                .response(ResponseDescription::new(
                    StatusCode::NOT_FOUND,
                    "Item not found",
                ))
                .response(
                    ResponseDescription::new(StatusCode::CONFLICT, "Taken").with_schema(conflict),
                ),
        );

        assert_eq!(statuses(&responses), ["200", "404", "409"]);
        assert_eq!(
            responses,
            json!({
                "200": { "description": "Successful Response", "content": json_content(json!({})) },
                "404": { "description": "Item not found" },
                "409": { "description": "Taken", "content": json_content(reference("Conflict")) }
            })
        );
    }

    #[test]
    fn a_declared_success_replaces_the_default_and_validation_comes_last() {
        let responses = responses(
            items()
                .body(Schema::string())
                .response(
                    ResponseDescription::new(StatusCode::CREATED, "Created")
                        .with_schema(Schema::integer()),
                )
                .response(ResponseDescription::new(StatusCode::NOT_FOUND, "Missing")),
        );

        assert_eq!(statuses(&responses), ["201", "404", "422"]);
        assert_eq!(
            responses["201"]["content"],
            json_content(json!({ "type": "integer" }))
        );
        assert_eq!(
            responses["422"]["content"],
            json_content(reference(HTTP_VALIDATION_ERROR))
        );
    }

    #[test]
    fn lists_each_description_and_body_a_status_can_carry_once() {
        let responses = responses(
            items()
                .body(Schema::string())
                .response(
                    ResponseDescription::new(StatusCode::UNPROCESSABLE_ENTITY, "Unprocessable")
                        .with_schema(Schema::string()),
                )
                .response(
                    ResponseDescription::new(StatusCode::BAD_REQUEST, "Bad header")
                        .with_schema(Schema::string()),
                )
                .response(
                    ResponseDescription::new(StatusCode::BAD_REQUEST, "Bad cookie")
                        .with_schema(Schema::integer()),
                )
                .response(
                    ResponseDescription::new(StatusCode::BAD_REQUEST, "Bad header")
                        .with_schema(Schema::string()),
                ),
        );

        assert_eq!(statuses(&responses), ["200", "422", "400"]);
        assert_eq!(
            responses["422"],
            json!({
                "description": "Unprocessable or Validation Error",
                "content": json_content(json!({
                    "anyOf": [{ "type": "string" }, reference(HTTP_VALIDATION_ERROR)]
                }))
            })
        );
        assert_eq!(
            responses["400"],
            json!({
                "description": "Bad header or Bad cookie",
                "content": json_content(json!({
                    "anyOf": [{ "type": "string" }, { "type": "integer" }]
                }))
            })
        );
    }

    #[test]
    fn a_dependency_declares_responses_of_the_route() {
        let responses = responses(
            items()
                .response(ResponseDescription::new(StatusCode::NOT_FOUND, "Missing"))
                .dependency(|dependency| {
                    dependency.response(ResponseDescription::new(
                        StatusCode::UNAUTHORIZED,
                        "Not authenticated",
                    ))
                }),
        );

        assert_eq!(statuses(&responses), ["200", "404", "401"]);
    }

    #[test]
    fn response_models_share_the_components_with_the_body() {
        let item = Schema::model("Item", [Field::new("name", Schema::string())]);
        let other_item = Schema::model("Item", [Field::new("name", Schema::integer())]);
        let shared = items().body(item.clone()).response(
            ResponseDescription::new(StatusCode::OK, "The item").with_schema(item.clone()),
        );
        let conflicting = items()
            .body(item)
            .response(ResponseDescription::new(StatusCode::OK, "The item").with_schema(other_item));

        let shared = document(shared).unwrap();

        assert_eq!(
            shared["paths"]["/items"]["post"]["responses"]["200"]["content"],
            json_content(reference("Item"))
        );
        assert_eq!(
            shared["components"]["schemas"]["Item"]["properties"]["name"]["type"],
            "string"
        );
        assert_eq!(
            document(conflicting).err(),
            Some(OpenApiError::NameConflict {
                name: "Item".to_owned(),
                method: Method::POST,
                path: "/items".to_owned()
            })
        );
    }
}
