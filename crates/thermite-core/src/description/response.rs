use http::StatusCode;

use crate::schema::Schema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseDescription {
    status: StatusCode,
    description: String,
    schema: Option<Schema>,
}

impl ResponseDescription {
    // without a schema the response has no body, like a 204
    #[must_use]
    pub fn new(status: StatusCode, description: impl Into<String>) -> Self {
        Self {
            status,
            description: description.into(),
            schema: None,
        }
    }

    #[must_use]
    pub fn with_schema(mut self, schema: Schema) -> Self {
        self.schema = Some(schema);
        self
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub(crate) fn description(&self) -> &str {
        &self.description
    }

    pub(crate) fn schema(&self) -> Option<&Schema> {
        self.schema.as_ref()
    }
}
