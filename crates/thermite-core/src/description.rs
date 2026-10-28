use std::fmt;

use http::{HeaderName, Method};
use serde_json::Value;

use crate::schema::{Field, Schema};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDescription {
    method: Method,
    path: String,
    parameters: Vec<Parameter>,
    body: Body,
}

// a second body is kept as a state of its own so that registering the route can report it
#[derive(Debug, Clone, PartialEq, Eq)]
enum Body {
    NotDeclared,
    Declared(Schema),
    DeclaredTwice,
}

impl RouteDescription {
    #[must_use]
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            parameters: Vec::new(),
            body: Body::NotDeclared,
        }
    }

    #[must_use]
    pub fn parameter(mut self, parameter: Parameter) -> Self {
        self.parameters.push(parameter);
        self
    }

    #[must_use]
    pub fn body(mut self, schema: Schema) -> Self {
        self.body = match self.body {
            Body::NotDeclared => Body::Declared(schema),
            Body::Declared(_) | Body::DeclaredTwice => Body::DeclaredTwice,
        };
        self
    }

    pub(crate) fn method(&self) -> &Method {
        &self.method
    }

    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    pub(crate) fn body_schema(&self) -> Option<&Schema> {
        match &self.body {
            Body::Declared(schema) => Some(schema),
            Body::NotDeclared | Body::DeclaredTwice => None,
        }
    }

    pub(crate) fn declares_two_bodies(&self) -> bool {
        self.body == Body::DeclaredTwice
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParameterLocation {
    Path,
    Query,
    Header,
    Cookie,
}

impl ParameterLocation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Query => "query",
            Self::Header => "header",
            Self::Cookie => "cookie",
        }
    }
}

impl fmt::Display for ParameterLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    location: ParameterLocation,
    field: Field,
    // parsed once for lookups and duplicate checks; None for other locations and for invalid names
    header_name: Option<HeaderName>,
}

impl Parameter {
    #[must_use]
    pub fn new(location: ParameterLocation, field: Field) -> Self {
        let header_name = match location {
            ParameterLocation::Header => HeaderName::from_bytes(field.name().as_bytes()).ok(),
            _ => None,
        };
        Self {
            location,
            field,
            header_name,
        }
    }

    #[must_use]
    pub fn path(name: impl Into<String>, schema: Schema) -> Self {
        Self::new(ParameterLocation::Path, Field::new(name, schema))
    }

    #[must_use]
    pub fn query(name: impl Into<String>, schema: Schema) -> Self {
        Self::new(ParameterLocation::Query, Field::new(name, schema))
    }

    // header names match case-insensitively, but errors report the name as declared
    #[must_use]
    pub fn header(name: impl Into<String>, schema: Schema) -> Self {
        Self::new(ParameterLocation::Header, Field::new(name, schema))
    }

    #[must_use]
    pub fn cookie(name: impl Into<String>, schema: Schema) -> Self {
        Self::new(ParameterLocation::Cookie, Field::new(name, schema))
    }

    #[must_use]
    pub fn optional(mut self) -> Self {
        self.field = self.field.optional();
        self
    }

    #[must_use]
    pub fn with_default(mut self, value: impl Into<Value>) -> Self {
        self.field = self.field.with_default(value);
        self
    }

    pub(crate) fn location(&self) -> ParameterLocation {
        self.location
    }

    pub(crate) fn field(&self) -> &Field {
        &self.field
    }

    pub(crate) fn header_name(&self) -> Option<&HeaderName> {
        self.header_name.as_ref()
    }
}
