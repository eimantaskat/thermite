use http::Method;

use crate::description::ParameterLocation;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RouteError {
    #[error("path `{path}` must start with `/`")]
    MissingLeadingSlash { path: String },
    #[error("prefix `{prefix}` of `{path}` must start with `/` and must not end with `/`")]
    InvalidPrefix { prefix: String, path: String },
    #[error("path `{path}` has an invalid parameter segment `{segment}`")]
    InvalidParameter { path: String, segment: String },
    #[error("path `{path}` uses the parameter `{name}` more than once")]
    DuplicateParameter { path: String, name: String },
    #[error("{location} parameter `{name}` of `{path}` is declared more than once")]
    DuplicateDeclaration {
        path: String,
        location: ParameterLocation,
        name: String,
    },
    #[error("path `{path}` has no segment `{{{name}}}` for its path parameter `{name}`")]
    UnknownPathParameter { path: String, name: String },
    #[error("path `{path}` has the segment `{{{name}}}` but no path parameter `{name}`")]
    UndeclaredPathParameter { path: String, name: String },
    #[error("path parameter `{name}` of `{path}` cannot be optional or have a default")]
    OptionalPathParameter { path: String, name: String },
    #[error("{location} parameter `{name}` of `{path}` must be {}", allowed_types(*.location))]
    UnsupportedParameterType {
        path: String,
        location: ParameterLocation,
        name: String,
    },
    #[error("header parameter `{name}` of `{path}` is not a valid header name")]
    InvalidHeaderName { path: String, name: String },
    #[error("`{path}` declares more than one body")]
    DuplicateBody { path: String },
    #[error("body of `{path}` has the field `{name}` more than once in one object")]
    DuplicateField { path: String, name: String },
    #[error("{method} `{path}` matches the same requests as the route `{existing}`")]
    Conflict {
        method: Method,
        path: String,
        existing: String,
    },
}

fn allowed_types(location: ParameterLocation) -> &'static str {
    match location {
        ParameterLocation::Path | ParameterLocation::Cookie => {
            "a string, integer, number or boolean"
        }
        ParameterLocation::Query | ParameterLocation::Header => {
            "a string, integer, number or boolean, or a list of those"
        }
    }
}
