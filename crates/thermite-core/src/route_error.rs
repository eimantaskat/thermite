use std::fmt;
use std::path::PathBuf;

use http::{Method, StatusCode};

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
    #[error("an API key of `{path}` is read from `{name}`, which is not a valid header name")]
    InvalidApiKeyHeader { path: String, name: String },
    #[error("`{path}` declares more than one body")]
    DuplicateBody { path: String },
    #[error("the form body of `{path}` must be an object, whose fields are the form's")]
    FormBodyNotObject { path: String },
    #[error("form field `{name}` of `{path}` must be a scalar or a file, or a list of them")]
    UnsupportedFormFieldType { path: String, name: String },
    #[error("the JSON body of `{path}` holds a file, which only a form body can")]
    FileInJsonBody { path: String },
    #[error("{location} of `{path}` has the field `{name}` more than once in one object")]
    DuplicateField {
        path: String,
        location: SchemaLocation,
        name: String,
    },
    #[error(
        "{location} of `{path}` has a default for `{name}` that is not a valid value of its schema"
    )]
    InvalidDefault {
        path: String,
        location: SchemaLocation,
        name: String,
    },
    #[error("{location} of `{path}` refers to the model `{name}` outside any model of that name")]
    UnresolvedReference {
        path: String,
        location: SchemaLocation,
        name: String,
    },
    #[error("{location} of `{path}` uses two different models named `{name}`")]
    ModelNameConflict {
        path: String,
        location: SchemaLocation,
        name: String,
    },
    #[error("the WebSocket route `{path}` is opened with GET, not {method}")]
    WebSocketMethod { method: Method, path: String },
    #[error("the WebSocket route `{path}` declares a body, which a handshake does not have")]
    WebSocketBody { path: String },
    #[error("the mount path `{path}` must start with `/` and hold no parameter or empty segment")]
    InvalidMountPath { path: String },
    #[error("static files are mounted at `{path}` more than once")]
    MountConflict { path: String },
    #[error(
        "the static files mounted at `{path}` come from `{}`, which is not a directory",
        .directory.display()
    )]
    StaticDirectory { path: String, directory: PathBuf },
    #[error("{method} `{path}` matches the same requests as the route `{existing}`")]
    Conflict {
        method: Method,
        path: String,
        existing: String,
    },
}

// what is wrong with a schema, wherever it is used
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SchemaProblem {
    #[error("has the field `{name}` more than once in one object")]
    DuplicateField { name: String },
    #[error("uses two different models named `{name}`")]
    ModelNameConflict { name: String },
    #[error("refers to the model `{name}` outside any model of that name")]
    UnresolvedReference { name: String },
    #[error("has a default for `{name}` that is not a valid value of its schema")]
    InvalidDefault { name: String },
}

impl SchemaProblem {
    pub(crate) fn at(self, location: SchemaLocation, path: &str) -> RouteError {
        let path = path.to_owned();
        match self {
            Self::DuplicateField { name } => RouteError::DuplicateField {
                path,
                location,
                name,
            },
            Self::ModelNameConflict { name } => RouteError::ModelNameConflict {
                path,
                location,
                name,
            },
            Self::UnresolvedReference { name } => RouteError::UnresolvedReference {
                path,
                location,
                name,
            },
            Self::InvalidDefault { name } => RouteError::InvalidDefault {
                path,
                location,
                name,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SchemaLocation {
    Parameter(ParameterLocation),
    Body,
    Response(StatusCode),
}

impl fmt::Display for SchemaLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parameter(location) => write!(formatter, "{location} parameter"),
            Self::Body => formatter.write_str("body"),
            Self::Response(status) => write!(formatter, "response {}", status.as_str()),
        }
    }
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
