use std::any::Any;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;

use crate::description::ParameterLocation;

#[derive(Debug, Default)]
pub struct Arguments {
    parameters: Vec<(ParameterLocation, Arc<str>, Value)>,
    body: Option<Value>,
    // extractors get the arguments shared, so the one that owns the decoded body takes it out
    decoded_body: Mutex<Option<Box<dyn Any + Send>>>,
}

impl Arguments {
    pub(crate) fn new(
        parameters: Vec<(ParameterLocation, Arc<str>, Value)>,
        body: Option<Value>,
        decoded_body: Option<Box<dyn Any + Send>>,
    ) -> Self {
        Self {
            parameters,
            body,
            decoded_body: Mutex::new(decoded_body),
        }
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

    // what the route's body decoder made, given out once; the body is then not also validated into `body`
    #[must_use]
    pub fn take_decoded_body(&self) -> Option<Box<dyn Any + Send>> {
        self.decoded_body
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    #[must_use]
    pub fn parameter(&self, location: ParameterLocation, name: &str) -> Option<&Value> {
        self.parameters
            .iter()
            .find(|(found_location, found_name, _)| {
                *found_location == location
                    && match location {
                        ParameterLocation::Header => found_name.eq_ignore_ascii_case(name),
                        _ => **found_name == *name,
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
                (ParameterLocation::Header, "X-Token".into(), json!("secret")),
                (ParameterLocation::Query, "Limit".into(), json!(5)),
            ],
            None,
            None,
        );

        assert_eq!(arguments.header("x-token"), Some(&json!("secret")));
        assert_eq!(arguments.query("Limit"), Some(&json!(5)));
        assert_eq!(arguments.query("limit"), None);
    }

    #[test]
    fn gives_out_the_decoded_body_once() {
        let arguments = Arguments::new(Vec::new(), None, Some(Box::new(7_i64)));

        let decoded = arguments.take_decoded_body();

        assert_eq!(
            decoded.and_then(|value| value.downcast::<i64>().ok()),
            Some(Box::new(7))
        );
        assert!(arguments.take_decoded_body().is_none());
        assert!(arguments.body().is_none());
    }
}
