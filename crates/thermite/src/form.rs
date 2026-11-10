use std::future::{Future, ready};

use serde::de::DeserializeOwned;
use thermite_core::{Arguments, Request, RouteDescription, Value};

use crate::extract::FromRequest;
use crate::model::{Model, ToSchema};
use crate::rejection::ModelMismatch;
use crate::upload::reading_files;

// FastAPI's `Annotated[Model, Form()]`: each field of the model is a field of the form, urlencoded or multipart
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Form<T>(pub T);

impl<T: Model + DeserializeOwned + Send> FromRequest for Form<T> {
    type Rejection = ModelMismatch;

    fn describe(description: RouteDescription) -> RouteDescription {
        description.form_body(T::schema())
    }

    fn from_request(
        _: &Request,
        arguments: &Arguments,
    ) -> impl Future<Output = Result<Self, ModelMismatch>> + Send {
        // the core answers 422 before this runs when a required field is missing
        let body = arguments.body().unwrap_or(&Value::Null);
        let read = reading_files(arguments.files(), || T::deserialize(body));
        ready(read.map(Self).map_err(ModelMismatch::new::<T>))
    }
}
