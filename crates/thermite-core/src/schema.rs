mod coerce;
mod json_schema;

use std::collections::HashSet;

use serde_json::Value;

pub(crate) use json_schema::{Components, JsonSchema, NameConflict, entries, title};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema(Kind);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    String,
    Integer,
    Number,
    Boolean,
    Array(Box<Schema>),
    // a named object is a model, documented once and referenced by name
    Object {
        name: Option<String>,
        fields: Vec<Field>,
    },
}

impl Schema {
    #[must_use]
    pub fn string() -> Self {
        Self(Kind::String)
    }

    #[must_use]
    pub fn integer() -> Self {
        Self(Kind::Integer)
    }

    #[must_use]
    pub fn number() -> Self {
        Self(Kind::Number)
    }

    #[must_use]
    pub fn boolean() -> Self {
        Self(Kind::Boolean)
    }

    #[must_use]
    pub fn array(items: Schema) -> Self {
        Self(Kind::Array(Box::new(items)))
    }

    #[must_use]
    pub fn object(fields: impl IntoIterator<Item = Field>) -> Self {
        Self(Kind::Object {
            name: None,
            fields: fields.into_iter().collect(),
        })
    }

    #[must_use]
    pub fn model(name: impl Into<String>, fields: impl IntoIterator<Item = Field>) -> Self {
        Self(Kind::Object {
            name: Some(name.into()),
            fields: fields.into_iter().collect(),
        })
    }

    pub(crate) fn is_scalar(&self) -> bool {
        !matches!(self.0, Kind::Array(_) | Kind::Object { .. })
    }

    pub(crate) fn is_scalar_or_list_of_scalars(&self) -> bool {
        match &self.0 {
            Kind::Array(items) => items.is_scalar(),
            _ => self.is_scalar(),
        }
    }

    pub(crate) fn duplicate_field(&self) -> Option<&str> {
        match &self.0 {
            Kind::Array(items) => items.duplicate_field(),
            Kind::Object { fields, .. } => {
                let mut seen = HashSet::new();
                fields
                    .iter()
                    .find(|field| !seen.insert(field.name.as_str()))
                    .map(|field| field.name.as_str())
                    .or_else(|| {
                        fields
                            .iter()
                            .find_map(|field| field.schema.duplicate_field())
                    })
            }
            _ => None,
        }
    }

    pub(crate) fn is_array(&self) -> bool {
        matches!(self.0, Kind::Array(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    name: String,
    schema: Schema,
    presence: Presence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Presence {
    Required,
    // like `x: int | None = None`: may be missing or null
    Optional,
    Default(Value),
}

impl Field {
    #[must_use]
    pub fn new(name: impl Into<String>, schema: Schema) -> Self {
        Self {
            name: name.into(),
            schema,
            presence: Presence::Required,
        }
    }

    #[must_use]
    pub fn optional(mut self) -> Self {
        self.presence = Presence::Optional;
        self
    }

    #[must_use]
    pub fn with_default(mut self, value: impl Into<Value>) -> Self {
        self.presence = Presence::Default(value.into());
        self
    }

    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn schema(&self) -> &Schema {
        &self.schema
    }

    pub(crate) fn is_required(&self) -> bool {
        self.presence == Presence::Required
    }

    pub(crate) fn matches_ignoring_name(&self, other: &Field) -> bool {
        self.schema == other.schema && self.presence == other.presence
    }
}
