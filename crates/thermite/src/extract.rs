mod parameters;
mod tuple;

use std::future::Future;

use thermite_core::{Arguments, Request, RouteDescription};

use crate::response::IntoResponse;

pub use parameters::{Cookie, Header, Path, Query};

pub trait FromRequest: Sized + Send {
    type Rejection: IntoResponse;

    // what the extractor declares here is validated by the core before any extractor runs
    #[must_use]
    fn describe(description: RouteDescription) -> RouteDescription {
        description
    }

    fn from_request(
        request: &Request,
        arguments: &Arguments,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}
