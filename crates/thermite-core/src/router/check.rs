use std::collections::HashSet;

use crate::description::{Parameter, ParameterLocation, RouteDescription};
use crate::route_error::RouteError;
use crate::routing::PathPattern;

pub(super) fn check_description(
    description: &RouteDescription,
    pattern: &PathPattern,
) -> Result<(), RouteError> {
    let path = description.path();
    let mut seen = HashSet::new();
    for parameter in description.parameters() {
        let name = parameter.field().name();
        if !seen.insert((parameter.location(), name)) {
            return Err(RouteError::DuplicateDeclaration {
                path: path.to_owned(),
                location: parameter.location(),
                name: name.to_owned(),
            });
        }
        check_parameter(parameter, pattern, path)?;
    }
    if let Some(name) = pattern
        .parameter_names()
        .find(|name| !seen.contains(&(ParameterLocation::Path, *name)))
    {
        return Err(RouteError::UndeclaredPathParameter {
            path: path.to_owned(),
            name: name.to_owned(),
        });
    }
    if let Some(name) = description
        .body_schema()
        .and_then(|schema| schema.duplicate_field())
    {
        return Err(RouteError::DuplicateField {
            path: path.to_owned(),
            name: name.to_owned(),
        });
    }
    Ok(())
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
        ParameterLocation::Path => field.schema().is_scalar(),
        ParameterLocation::Query => field.schema().is_scalar_or_list_of_scalars(),
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

    fn check(description: &RouteDescription) -> Result<(), RouteError> {
        check_description(
            description,
            &PathPattern::parse(description.path()).unwrap(),
        )
    }

    fn items(path: &str) -> RouteDescription {
        RouteDescription::new(Method::GET, path)
    }

    #[test]
    fn accepts_scalar_path_parameters_and_lists_in_the_query() {
        let description = items("/items/{item_id}")
            .parameter(Parameter::path("item_id", Schema::integer()))
            .parameter(Parameter::query("item_id", Schema::string()))
            .parameter(Parameter::query("tag", Schema::array(Schema::string())).optional())
            .body(Schema::object([
                Field::new("a", Schema::object([Field::new("a", Schema::string())])),
                Field::new("b", Schema::string()),
            ]));

        assert_eq!(check(&description), Ok(()));
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
    fn rejects_parameter_types_a_url_cannot_carry() {
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
                items("/items").parameter(Parameter::query("filter", Schema::array(object))),
                ParameterLocation::Query,
                "filter",
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
