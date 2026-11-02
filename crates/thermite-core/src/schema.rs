mod coerce;
mod json_schema;

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::validation::Location;

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
    // a model repeated inside itself, standing for the nearest enclosing model of that name
    Reference(String),
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

    #[must_use]
    pub fn reference(name: impl Into<String>) -> Self {
        Self(Kind::Reference(name.into()))
    }

    pub(crate) fn is_scalar(&self) -> bool {
        !matches!(
            self.0,
            Kind::Array(_) | Kind::Object { .. } | Kind::Reference(_)
        )
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

    // a reference names its model only by name, so one body may hold one model per name
    pub(crate) fn conflicting_model_name(&self) -> Option<&str> {
        self.conflicting_model_name_among(&mut HashMap::new())
    }

    fn conflicting_model_name_among<'schema>(
        &'schema self,
        seen: &mut HashMap<&'schema str, &'schema Schema>,
    ) -> Option<&'schema str> {
        match &self.0 {
            Kind::Array(items) => items.conflicting_model_name_among(seen),
            Kind::Object { name, fields } => {
                if let Some(name) = name {
                    // a model seen before had its fields walked then
                    if let Some(known) = seen.insert(name, self) {
                        return (known != self).then_some(name.as_str());
                    }
                }
                fields
                    .iter()
                    .find_map(|field| field.schema.conflicting_model_name_among(seen))
            }
            _ => None,
        }
    }

    pub(crate) fn unresolved_reference(&self) -> Option<&str> {
        self.unresolved_reference_within(&Models::default())
    }

    fn unresolved_reference_within(&self, models: &Models<'_>) -> Option<&str> {
        match &self.0 {
            Kind::Array(items) => items.unresolved_reference_within(models),
            Kind::Object { name, fields } => {
                let models = models.within(name.as_deref(), fields);
                fields
                    .iter()
                    .find_map(|field| field.schema.unresolved_reference_within(&models))
            }
            Kind::Reference(name) if models.find(name).is_none() => Some(name),
            _ => None,
        }
    }

    pub(crate) fn invalid_default(&self) -> Option<&str> {
        self.invalid_default_within(&Models::default())
    }

    fn invalid_default_within(&self, models: &Models<'_>) -> Option<&str> {
        match &self.0 {
            Kind::Array(items) => items.invalid_default_within(models),
            Kind::Object { name, fields } => {
                let models = models.within(name.as_deref(), fields);
                fields
                    .iter()
                    .find_map(|field| field.invalid_default_within(&models))
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

    pub(crate) fn invalid_default(&self) -> Option<&str> {
        self.invalid_default_within(&Models::default())
    }

    // a default skips validation on its way to serde, so it must already be what validation returns
    fn invalid_default_within(&self, models: &Models<'_>) -> Option<&str> {
        if let Presence::Default(default) = &self.presence
            && !self
                .schema
                .validate_within(default.clone(), &Location::root("default"), models)
                .is_ok_and(|validated| validated == *default)
        {
            return Some(&self.name);
        }
        self.schema.invalid_default_within(models)
    }

    pub(crate) fn matches_ignoring_name(&self, other: &Field) -> bool {
        self.schema == other.schema && self.presence == other.presence
    }
}

// the models a value sits in, innermost first, so that a reference finds the model it repeats
#[derive(Debug, Clone, Copy, Default)]
struct Models<'schema> {
    // None for an object without a name, which no reference can stand for
    innermost: Option<(&'schema str, &'schema [Field])>,
    outer: Option<&'schema Models<'schema>>,
}

impl<'schema> Models<'schema> {
    fn within(&'schema self, name: Option<&'schema str>, fields: &'schema [Field]) -> Self {
        Self {
            innermost: name.map(|name| (name, fields)),
            outer: Some(self),
        }
    }

    fn find(&self, name: &str) -> Option<&'schema [Field]> {
        let mut models = Some(self);
        while let Some(current) = models {
            if let Some((model, fields)) = current.innermost
                && model == name
            {
                return Some(fields);
            }
            models = current.outer;
        }
        None
    }
}
