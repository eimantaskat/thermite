use std::fmt;

use http::Method;
use serde_json::Value;

use crate::schema::{Field, Schema};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDescription {
    method: Method,
    path: String,
    parameters: Vec<Parameter>,
    body: Option<Schema>,
}

impl RouteDescription {
    #[must_use]
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            parameters: Vec::new(),
            body: None,
        }
    }

    #[must_use]
    pub fn parameter(mut self, parameter: Parameter) -> Self {
        self.parameters.push(parameter);
        self
    }

    #[must_use]
    pub fn body(mut self, schema: Schema) -> Self {
        self.body = Some(schema);
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
        self.body.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParameterLocation {
    Path,
    Query,
}

impl ParameterLocation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Query => "query",
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
}

impl Parameter {
    #[must_use]
    pub fn path(name: impl Into<String>, schema: Schema) -> Self {
        Self::new(ParameterLocation::Path, name, schema)
    }

    #[must_use]
    pub fn query(name: impl Into<String>, schema: Schema) -> Self {
        Self::new(ParameterLocation::Query, name, schema)
    }

    fn new(location: ParameterLocation, name: impl Into<String>, schema: Schema) -> Self {
        Self {
            location,
            field: Field::new(name, schema),
        }
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
}
