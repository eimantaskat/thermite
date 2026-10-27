use serde_json::Value;

use crate::description::ParameterLocation;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Arguments {
    parameters: Vec<(ParameterLocation, String, Value)>,
    body: Option<Value>,
}

impl Arguments {
    pub(crate) fn new(
        parameters: Vec<(ParameterLocation, String, Value)>,
        body: Option<Value>,
    ) -> Self {
        Self { parameters, body }
    }

    #[must_use]
    pub fn path(&self, name: &str) -> Option<&Value> {
        self.parameter(ParameterLocation::Path, name)
    }

    #[must_use]
    pub fn query(&self, name: &str) -> Option<&Value> {
        self.parameter(ParameterLocation::Query, name)
    }

    #[must_use]
    pub fn body(&self) -> Option<&Value> {
        self.body.as_ref()
    }

    fn parameter(&self, location: ParameterLocation, name: &str) -> Option<&Value> {
        self.parameters
            .iter()
            .find(|(found_location, found_name, _)| {
                *found_location == location && found_name == name
            })
            .map(|(_, _, value)| value)
    }
}
