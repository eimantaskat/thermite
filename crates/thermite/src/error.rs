use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

use thermite_core::{HttpError, Response, attach_error};

use crate::response::{IntoResponse, internal_server_error};

// like anyhow::Error, it does not implement std's Error itself, so that `?` converts any error into it
pub struct Error(Arc<dyn StdError + Send + Sync>);

impl<E: StdError + Send + Sync + 'static> From<E> for Error {
    fn from(error: E) -> Self {
        Self(Arc::new(error))
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

// an HttpError keeps its own status and detail, as a raised HTTPException does
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        if let Some(error) = self.0.downcast_ref::<HttpError>() {
            return error.clone().into_response();
        }
        let mut response = internal_server_error();
        attach_error(&mut response, self.0);
        response
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        HttpError::into_response(self)
    }
}
