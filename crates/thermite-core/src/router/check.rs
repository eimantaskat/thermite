use std::collections::HashSet;

use super::path::PathPattern;
use crate::description::{
    Body, Declarations, Parameter, ParameterLocation, ResponseDescription, RouteDescription,
};
use crate::route_error::{RouteError, SchemaLocation};
use crate::schema::Schema;

// what a route reads from each request, in the order FastAPI reports its errors
#[derive(Debug, Default)]
pub(super) struct Inputs {
    pub(super) parameters: Vec<Parameter>,
    pub(super) body: Option<Schema>,
    // like FastAPI, body errors follow the values of the group that declared the body
    pub(super) body_position: usize,
}

pub(super) fn check_description(
    description: RouteDescription,
    pattern: &PathPattern,
) -> Result<(Inputs, Vec<ResponseDescription>), RouteError> {
    let path = description.path().to_owned();
    let (declarations, responses) = description.into_parts();
    let mut inputs = Inputs::default();
    flatten(declarations, &path, &mut inputs)?;
    for parameter in &inputs.parameters {
        check_parameter(parameter, pattern, &path)?;
    }
    if let Some(name) = pattern.parameter_names().find(|name| {
        !inputs.parameters.iter().any(|parameter| {
            parameter.location() == ParameterLocation::Path && parameter.field().name() == *name
        })
    }) {
        return Err(RouteError::UndeclaredPathParameter {
            path,
            name: name.to_owned(),
        });
    }
    if let Some(body) = &inputs.body {
        check_schema(body, SchemaLocation::Body, &path)?;
    }
    for response in &responses {
        if let Some(schema) = response.schema() {
            check_schema(schema, SchemaLocation::Response(response.status()), &path)?;
        }
    }
    Ok((inputs, responses))
}

// responses are never validated, but a wrong schema would still describe them wrongly
fn check_schema(schema: &Schema, location: SchemaLocation, path: &str) -> Result<(), RouteError> {
    if let Some(name) = schema.duplicate_field() {
        return Err(RouteError::DuplicateField {
            path: path.to_owned(),
            location,
            name: name.to_owned(),
        });
    }
    if let Some(name) = schema.conflicting_model_name() {
        return Err(RouteError::ModelNameConflict {
            path: path.to_owned(),
            location,
            name: name.to_owned(),
        });
    }
    // before the defaults, which an unresolved reference would let through unchecked
    if let Some(name) = schema.unresolved_reference() {
        return Err(RouteError::UnresolvedReference {
            path: path.to_owned(),
            location,
            name: name.to_owned(),
        });
    }
    if let Some(name) = schema.invalid_default() {
        return Err(RouteError::InvalidDefault {
            path: path.to_owned(),
            location,
            name: name.to_owned(),
        });
    }
    Ok(())
}

// like FastAPI, a group's dependencies come before its own values, depth first
fn flatten(declarations: Declarations, path: &str, inputs: &mut Inputs) -> Result<(), RouteError> {
    let Declarations {
        mut parameters,
        body,
        dependencies,
    } = declarations;
    for dependency in dependencies {
        flatten(dependency, path, inputs)?;
    }
    parameters.sort_by_key(|parameter| parameter.location().rank());
    let mut own = HashSet::new();
    for parameter in parameters {
        let duplicate = || RouteError::DuplicateDeclaration {
            path: path.to_owned(),
            location: parameter.location(),
            name: parameter.field().name().to_owned(),
        };
        if !own.insert((parameter.location(), parameter.declared_name().to_owned())) {
            return Err(duplicate());
        }
        // a dependency shared by several others is declared once by each, and read once
        let earlier = inputs.parameters.iter().find(|earlier| {
            earlier.location() == parameter.location()
                && earlier.declared_name() == parameter.declared_name()
        });
        match earlier {
            Some(earlier) if earlier.declares_the_same_as(&parameter) => {}
            Some(_) => return Err(duplicate()),
            None => inputs.parameters.push(parameter),
        }
    }
    match (body, &inputs.body) {
        (Body::NotDeclared, _) => Ok(()),
        (Body::Declared(schema), None) => {
            inputs.body = Some(schema);
            inputs.body_position = inputs.parameters.len();
            Ok(())
        }
        (Body::Declared(schema), Some(earlier)) if schema == *earlier => Ok(()),
        (Body::Declared(_), Some(_)) | (Body::DeclaredTwice, _) => Err(RouteError::DuplicateBody {
            path: path.to_owned(),
        }),
    }
}

fn check_parameter(
    parameter: &Parameter,
    pattern: &PathPattern,
    path: &str,
) -> Result<(), RouteError> {
    let field = parameter.field();
    let name = field.name().to_owned();
    let path = path.to_owned();
    let supported = match parameter.location() {
        ParameterLocation::Path if !pattern.has_parameter(&name) => {
            return Err(RouteError::UnknownPathParameter { path, name });
        }
        ParameterLocation::Path if !field.is_required() => {
            return Err(RouteError::OptionalPathParameter { path, name });
        }
        ParameterLocation::Header if parameter.header_name().is_none() => {
            return Err(RouteError::InvalidHeaderName { path, name });
        }
        // like Starlette, a cookie name holds one value, so a list could never validate
        ParameterLocation::Path | ParameterLocation::Cookie => field.schema().is_scalar(),
        ParameterLocation::Query | ParameterLocation::Header => {
            field.schema().is_scalar_or_list_of_scalars()
        }
    };
    if !supported {
        return Err(RouteError::UnsupportedParameterType {
            path,
            location: parameter.location(),
            name,
        });
    }
    if field.invalid_default().is_some() {
        return Err(RouteError::InvalidDefault {
            path,
            location: SchemaLocation::Parameter(parameter.location()),
            name,
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use http::{Method, StatusCode};

    use super::*;
    use crate::schema::{Field, Schema};

    fn flattened(description: &RouteDescription) -> Result<Inputs, RouteError> {
        check_description(
            description.clone(),
            &PathPattern::parse(description.path()).unwrap(),
        )
        .map(|(inputs, _)| inputs)
    }

    fn check(description: &RouteDescription) -> Result<(), RouteError> {
        flattened(description).map(|_| ())
    }

    fn items(path: &str) -> RouteDescription {
        RouteDescription::new(Method::GET, path)
    }

    #[test]
    fn accepts_scalars_everywhere_and_lists_in_the_query_and_headers() {
        let description = items("/items/{item_id}")
            .parameter(Parameter::path("item_id", Schema::integer()))
            .parameter(Parameter::query("item_id", Schema::string()))
            .parameter(Parameter::query("tag", Schema::array(Schema::string())).optional())
            .parameter(Parameter::header("X-Tag", Schema::array(Schema::string())))
            .parameter(Parameter::cookie("session", Schema::string()))
            .body(Schema::object([
                Field::new("a", Schema::object([Field::new("a", Schema::string())])),
                Field::new("b", Schema::string()),
            ]));

        assert_eq!(check(&description), Ok(()));
    }

    #[test]
    fn orders_dependencies_first_then_each_group_by_location() {
        let description = items("/items/{id}")
            .parameter(Parameter::query("q", Schema::string()))
            .parameter(Parameter::path("id", Schema::integer()))
            .dependency(|dependency| {
                dependency
                    .parameter(Parameter::header("x-key", Schema::string()))
                    .dependency(|inner| {
                        inner.parameter(Parameter::cookie("session", Schema::string()))
                    })
                    .parameter(Parameter::query("limit", Schema::integer()))
            });

        let names = flattened(&description)
            .unwrap()
            .parameters
            .iter()
            .map(|parameter| parameter.field().name().to_owned())
            .collect::<Vec<_>>();

        assert_eq!(names, ["session", "limit", "x-key", "id", "q"]);
    }

    #[test]
    fn reads_a_value_declared_the_same_way_by_several_groups_once() {
        let paging = |dependency: RouteDescription| {
            dependency.parameter(Parameter::query("limit", Schema::integer()))
        };
        let description = items("/items")
            .dependency(paging)
            .dependency(|dependency| {
                dependency
                    .parameter(Parameter::header("X-Key", Schema::string()))
                    .body(Schema::integer())
            })
            .dependency(paging)
            .parameter(Parameter::header("x-key", Schema::string()))
            .body(Schema::integer());

        let inputs = flattened(&description).unwrap();

        assert_eq!(
            inputs.parameters,
            [
                Parameter::query("limit", Schema::integer()),
                Parameter::header("X-Key", Schema::string())
            ]
        );
        assert_eq!(inputs.body, Some(Schema::integer()));
    }

    #[test]
    fn rejects_groups_that_declare_the_same_name_differently() {
        let parameter = items("/items")
            .dependency(|dependency| dependency.parameter(Parameter::query("q", Schema::string())))
            .parameter(Parameter::query("q", Schema::string()).optional());
        let body = items("/items")
            .dependency(|dependency| dependency.body(Schema::string()))
            .body(Schema::integer());

        assert_eq!(
            check(&parameter),
            Err(RouteError::DuplicateDeclaration {
                path: "/items".to_owned(),
                location: ParameterLocation::Query,
                name: "q".to_owned()
            })
        );
        assert_eq!(
            check(&body),
            Err(RouteError::DuplicateBody {
                path: "/items".to_owned()
            })
        );
    }

    #[test]
    fn rejects_a_parameter_declared_twice_in_the_same_place() {
        let description = items("/items")
            .parameter(Parameter::query("q", Schema::string()))
            .parameter(Parameter::query("q", Schema::integer()));

        assert_eq!(
            check(&description),
            Err(RouteError::DuplicateDeclaration {
                path: "/items".to_owned(),
                location: ParameterLocation::Query,
                name: "q".to_owned()
            })
        );
    }

    #[test]
    fn rejects_header_names_that_differ_only_in_case() {
        let description = items("/items")
            .parameter(Parameter::header("x-token", Schema::string()))
            .parameter(Parameter::query("X-Token", Schema::string()))
            .parameter(Parameter::header("X-Token", Schema::string()));

        assert_eq!(
            check(&description),
            Err(RouteError::DuplicateDeclaration {
                path: "/items".to_owned(),
                location: ParameterLocation::Header,
                name: "X-Token".to_owned()
            })
        );
    }

    #[test]
    fn rejects_a_header_name_http_cannot_carry() {
        let description = items("/items").parameter(Parameter::header("x token", Schema::string()));

        assert_eq!(
            check(&description),
            Err(RouteError::InvalidHeaderName {
                path: "/items".to_owned(),
                name: "x token".to_owned()
            })
        );
    }

    #[test]
    fn rejects_a_path_parameter_missing_from_the_pattern() {
        let description =
            items("/items/{id}").parameter(Parameter::path("item_id", Schema::integer()));

        assert_eq!(
            check(&description),
            Err(RouteError::UnknownPathParameter {
                path: "/items/{id}".to_owned(),
                name: "item_id".to_owned()
            })
        );
    }

    #[test]
    fn rejects_a_pattern_parameter_without_a_declaration() {
        let description = items("/users/{user_id}/items/{item_id}")
            .parameter(Parameter::path("user_id", Schema::integer()))
            .parameter(Parameter::query("item_id", Schema::integer()));

        assert_eq!(
            check(&description),
            Err(RouteError::UndeclaredPathParameter {
                path: "/users/{user_id}/items/{item_id}".to_owned(),
                name: "item_id".to_owned()
            })
        );
    }

    #[test]
    fn rejects_an_optional_path_parameter() {
        for parameter in [
            Parameter::path("id", Schema::integer()).optional(),
            Parameter::path("id", Schema::integer()).with_default(1),
        ] {
            assert_eq!(
                check(&items("/items/{id}").parameter(parameter)),
                Err(RouteError::OptionalPathParameter {
                    path: "/items/{id}".to_owned(),
                    name: "id".to_owned()
                })
            );
        }
    }

    #[test]
    fn rejects_parameter_types_a_request_cannot_carry() {
        let object = Schema::object([Field::new("a", Schema::string())]);
        let cases = [
            (
                items("/items/{id}")
                    .parameter(Parameter::path("id", Schema::array(Schema::integer()))),
                ParameterLocation::Path,
                "id",
            ),
            (
                items("/items").parameter(Parameter::query("filter", object.clone())),
                ParameterLocation::Query,
                "filter",
            ),
            (
                items("/items")
                    .parameter(Parameter::query("filter", Schema::array(object.clone()))),
                ParameterLocation::Query,
                "filter",
            ),
            (
                items("/items").parameter(Parameter::query("node", Schema::reference("Node"))),
                ParameterLocation::Query,
                "node",
            ),
            (
                items("/items").parameter(Parameter::header("x-filter", object)),
                ParameterLocation::Header,
                "x-filter",
            ),
            (
                items("/items").parameter(Parameter::cookie(
                    "session",
                    Schema::array(Schema::string()),
                )),
                ParameterLocation::Cookie,
                "session",
            ),
        ];

        for (description, location, name) in cases {
            assert_eq!(
                check(&description),
                Err(RouteError::UnsupportedParameterType {
                    path: description.path().to_owned(),
                    location,
                    name: name.to_owned()
                })
            );
        }
    }

    #[test]
    fn rejects_a_second_body() {
        let two = items("/items")
            .body(Schema::string())
            .body(Schema::integer());
        let three = two.clone().body(Schema::boolean());

        for description in [two, three] {
            assert_eq!(
                check(&description),
                Err(RouteError::DuplicateBody {
                    path: "/items".to_owned()
                })
            );
        }
    }

    #[test]
    fn rejects_a_body_object_with_the_same_field_twice() {
        let nested = Schema::array(Schema::object([
            Field::new("id", Schema::integer()),
            Field::new("id", Schema::string()),
        ]));
        let description = items("/items").body(Schema::object([Field::new("items", nested)]));

        assert_eq!(
            check(&description),
            Err(RouteError::DuplicateField {
                path: "/items".to_owned(),
                location: SchemaLocation::Body,
                name: "id".to_owned()
            })
        );
    }

    // the core fills a default in without validating it, so serde would be the first to see a wrong one
    #[test]
    fn rejects_a_default_that_validation_would_not_return_unchanged() {
        let parameter = items("/items")
            .parameter(Parameter::query("limit", Schema::integer()).with_default("ten"));
        let coercible = items("/items")
            .parameter(Parameter::query("limit", Schema::integer()).with_default("20"));
        let nested = items("/items").body(Schema::object([Field::new(
            "tags",
            Schema::array(Schema::object([
                Field::new("label", Schema::string()).with_default(3)
            ])),
        )]));
        let query = SchemaLocation::Parameter(ParameterLocation::Query);

        assert_eq!(
            check(&parameter).unwrap_err().to_string(),
            "query parameter of `/items` has a default for `limit` that is not a valid value of its schema"
        );
        for (description, location, name) in [
            (parameter, query, "limit"),
            (coercible, query, "limit"),
            (nested, SchemaLocation::Body, "label"),
        ] {
            assert_eq!(
                check(&description),
                Err(RouteError::InvalidDefault {
                    path: "/items".to_owned(),
                    location,
                    name: name.to_owned()
                })
            );
        }
    }

    #[test]
    fn accepts_defaults_of_their_own_schema() {
        let description = items("/items")
            .parameter(Parameter::query("limit", Schema::integer()).with_default(20))
            .parameter(Parameter::query("tag", Schema::array(Schema::string())).with_default(["a"]))
            .body(Schema::object([Field::new(
                "tags",
                Schema::array(Schema::object([Field::new("label", Schema::string())])),
            )
            .with_default(serde_json::json!([{"label": "new"}]))]));

        assert_eq!(check(&description), Ok(()));
    }

    fn tree(default: serde_json::Value) -> Schema {
        Schema::model(
            "Node",
            [
                Field::new("label", Schema::string()),
                Field::new("children", Schema::array(Schema::reference("Node")))
                    .with_default(default),
            ],
        )
    }

    #[test]
    fn rejects_a_reference_outside_a_model_of_its_name() {
        let bare = items("/items").body(Schema::reference("Node"));
        let elsewhere = items("/items").body(Schema::model(
            "Tree",
            [Field::new("root", Schema::reference("Node"))],
        ));
        let unnamed = items("/items").body(Schema::object([Field::new(
            "root",
            Schema::reference("Node"),
        )]));

        for description in [bare, elsewhere, unnamed] {
            assert_eq!(
                check(&description),
                Err(RouteError::UnresolvedReference {
                    path: "/items".to_owned(),
                    location: SchemaLocation::Body,
                    name: "Node".to_owned()
                })
            );
        }
    }

    #[test]
    fn checks_defaults_inside_a_model_that_contains_itself() {
        let valid = items("/items").body(Schema::array(tree(
            serde_json::json!([{"label": "leaf", "children": []}]),
        )));
        let invalid = items("/items").body(tree(serde_json::json!([{"label": 1, "children": []}])));

        assert_eq!(check(&valid), Ok(()));
        assert_eq!(
            check(&invalid),
            Err(RouteError::InvalidDefault {
                path: "/items".to_owned(),
                location: SchemaLocation::Body,
                name: "children".to_owned()
            })
        );
    }

    #[test]
    fn rejects_two_different_models_of_one_name_in_a_body() {
        let tag = Schema::model("Tag", [Field::new("label", Schema::string())]);
        let other_tag = Schema::model("Tag", [Field::new("label", Schema::integer())]);
        let conflicting = items("/items").body(Schema::object([
            Field::new("main", tag.clone()),
            Field::new("others", Schema::array(other_tag)),
        ]));
        let repeated = items("/items").body(Schema::object([
            Field::new("main", tag.clone()),
            Field::new("others", Schema::array(tag)),
        ]));

        assert_eq!(
            check(&conflicting),
            Err(RouteError::ModelNameConflict {
                path: "/items".to_owned(),
                location: SchemaLocation::Body,
                name: "Tag".to_owned()
            })
        );
        assert_eq!(check(&repeated), Ok(()));
    }

    #[test]
    fn checks_the_schema_of_each_response() {
        let twice = Schema::object([
            Field::new("id", Schema::integer()),
            Field::new("id", Schema::integer()),
        ]);
        let duplicate = items("/items").response(
            ResponseDescription::new(StatusCode::NOT_FOUND, "Missing").with_schema(twice),
        );
        let unresolved = items("/items").response(
            ResponseDescription::new(StatusCode::OK, "A node")
                .with_schema(Schema::reference("Node")),
        );
        let wrong_default = items("/items").response(
            ResponseDescription::new(StatusCode::CREATED, "Created").with_schema(Schema::object([
                Field::new("label", Schema::string()).with_default(3),
            ])),
        );

        let duplicate = check(&duplicate).unwrap_err();

        assert_eq!(
            duplicate,
            RouteError::DuplicateField {
                path: "/items".to_owned(),
                location: SchemaLocation::Response(StatusCode::NOT_FOUND),
                name: "id".to_owned()
            }
        );
        assert_eq!(
            duplicate.to_string(),
            "response 404 of `/items` has the field `id` more than once in one object"
        );
        assert_eq!(
            check(&unresolved),
            Err(RouteError::UnresolvedReference {
                path: "/items".to_owned(),
                location: SchemaLocation::Response(StatusCode::OK),
                name: "Node".to_owned()
            })
        );
        assert_eq!(
            check(&wrong_default).unwrap_err().to_string(),
            "response 201 of `/items` has a default for `label` that is not a valid value of its schema"
        );
    }
}
