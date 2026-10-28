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
    pub fn header(&self, name: &str) -> Option<&Value> {
        self.parameter(ParameterLocation::Header, name)
    }

    #[must_use]
    pub fn cookie(&self, name: &str) -> Option<&Value> {
        self.parameter(ParameterLocation::Cookie, name)
    }

    #[must_use]
    pub fn body(&self) -> Option<&Value> {
        self.body.as_ref()
    }

    #[must_use]
    pub fn parameter(&self, location: ParameterLocation, name: &str) -> Option<&Value> {
        self.parameters
            .iter()
            .find(|(found_location, found_name, _)| {
                *found_location == location
                    && match location {
                        ParameterLocation::Header => found_name.eq_ignore_ascii_case(name),
                        _ => found_name == name,
                    }
            })
            .map(|(_, _, value)| value)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn finds_headers_in_any_case_and_other_names_exactly() {
        let arguments = Arguments::new(
            vec![
                (
                    ParameterLocation::Header,
                    "X-Token".to_owned(),
                    json!("secret"),
                ),
                (ParameterLocation::Query, "Limit".to_owned(), json!(5)),
            ],
            None,
        );

        assert_eq!(arguments.header("x-token"), Some(&json!("secret")));
        assert_eq!(arguments.query("Limit"), Some(&json!(5)));
        assert_eq!(arguments.query("limit"), None);
    }
}
