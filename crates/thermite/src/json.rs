use std::any::type_name;
use std::future::{Future, ready};

use serde::Serialize;
use serde::de::DeserializeOwned;
use thermite_core::{Arguments, Request, Response, RouteDescription, Value};

use crate::extract::FromRequest;
use crate::model::ToSchema;
use crate::rejection::ModelMismatch;
use crate::response::{IntoResponse, internal_server_error, with_content_type};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Json<T>(pub T);

impl<T: ToSchema + DeserializeOwned + Send> FromRequest for Json<T> {
    type Rejection = ModelMismatch;

    fn describe(description: RouteDescription) -> RouteDescription {
        description.body(T::schema())
    }

    fn from_request(
        _: &Request,
        arguments: &Arguments,
    ) -> impl Future<Output = Result<Self, ModelMismatch>> + Send {
        // the core answers 422 before this runs when the declared body is missing
        let body = arguments.body().unwrap_or(&Value::Null);
        ready(
            T::deserialize(body)
                .map(Self)
                .map_err(ModelMismatch::new::<T>),
        )
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        match serde_json::to_vec(&self.0) {
            Ok(body) => with_content_type(Response::new(body.into()), "application/json"),
            Err(error) => {
                tracing::error!("`{}` does not serialize to JSON: {error}", type_name::<T>());
                internal_server_error()
            }
        }
    }
}
