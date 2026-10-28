use std::collections::HashSet;

use super::path::PathPattern;
use crate::description::{Body, Declarations, Parameter, ParameterLocation, RouteDescription};
use crate::route_error::RouteError;
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
) -> Result<Inputs, RouteError> {
    let path = description.path().to_owned();
    let mut inputs = Inputs::default();
    flatten(description.into_declarations(), &path, &mut inputs)?;
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
    if let Some(name) = inputs
        .body
        .as_ref()
        .and_then(|schema| schema.duplicate_field())
    {
        return Err(RouteError::DuplicateField {
            path,
            name: name.to_owned(),
        });
    }
    Ok(inputs)
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
    if supported {
        Ok(())
    } else {
        Err(RouteError::UnsupportedParameterType {
            path,
            location: parameter.location(),
            name,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use http::Method;

    use super::*;
    use crate::schema::{Field, Schema};

    fn flattened(description: &RouteDescription) -> Result<Inputs, RouteError> {
        check_description(
            description.clone(),
            &PathPattern::parse(description.path()).unwrap(),
        )
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
                name: "id".to_owned()
            })
        );
    }
}
