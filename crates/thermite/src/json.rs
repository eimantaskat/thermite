use std::any::{Any, type_name};
use std::future::{Future, ready};
use std::marker::PhantomData;

use http::StatusCode;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thermite_core::{
    Arguments, BodyDecoder, Request, Response, ResponseDescription, RouteDescription, Value,
};

use crate::extract::FromRequest;
use crate::model::ToSchema;
use crate::rejection::ModelMismatch;
use crate::response::{
    IntoResponse, SUCCESSFUL_RESPONSE, internal_server_error, with_content_type,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Json<T>(pub T);

impl<T: ToSchema + DeserializeOwned + Send + 'static> FromRequest for Json<T> {
    type Rejection = ModelMismatch;

    fn describe(description: RouteDescription) -> RouteDescription {
        description.body_with_decoder(T::schema(), Serde::<T>(PhantomData))
    }

    fn from_request(
        _: &Request,
        arguments: &Arguments,
    ) -> impl Future<Output = Result<Self, ModelMismatch>> + Send {
        if let Some(decoded) = arguments
            .take_decoded_body()
            .and_then(|decoded| decoded.downcast::<T>().ok())
        {
            return ready(Ok(Self(*decoded)));
        }
        // the core answers 422 before this runs when the declared body is missing
        let body = arguments.body().unwrap_or(&Value::Null);
        ready(
            T::deserialize(body)
                .map(Self)
                .map_err(ModelMismatch::new::<T>),
        )
    }
}

// serde is never more lenient than the schema `derive(Model)` writes, so what it accepts validation would too
struct Serde<T>(PhantomData<fn() -> T>);

impl<T: DeserializeOwned + Send + 'static> BodyDecoder for Serde<T> {
    fn decode(&self, body: &[u8]) -> Option<Box<dyn Any + Send>> {
        let decoded: T = serde_json::from_slice(body).ok()?;
        Some(Box::new(decoded))
    }
}

impl<T: Serialize + ToSchema> IntoResponse for Json<T> {
    fn describe() -> Vec<ResponseDescription> {
        vec![ResponseDescription::new(StatusCode::OK, SUCCESSFUL_RESPONSE).with_schema(T::schema())]
    }

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
