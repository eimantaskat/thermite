use std::any::type_name;

use thermite_core::Response;

use crate::response::{IntoResponse, internal_server_error};

// the core validated the values, so this means the Model and the serde impl of a type disagree
#[derive(Debug, thiserror::Error)]
#[error("validated values do not deserialize into `{type_name}`: {error}")]
pub struct ModelMismatch {
    type_name: &'static str,
    error: serde_json::Error,
}

impl ModelMismatch {
    pub(crate) fn new<T>(error: serde_json::Error) -> Self {
        Self {
            type_name: type_name::<T>(),
            error,
        }
    }
}

impl IntoResponse for ModelMismatch {
    fn into_response(self) -> Response {
        tracing::error!("{self}");
        internal_server_error()
    }
}
