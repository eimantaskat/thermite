mod parameters;
mod tuple;

use std::future::Future;

use thermite_core::{Arguments, Request, RouteDescription};

use crate::requirements::Requirements;
use crate::response::{IntoResponse, describe_responses};

pub use parameters::{Cookie, Header, Path, Query};

pub trait FromRequest: Sized + Send {
    type Rejection: IntoResponse;

    // what the extractor declares here is validated by the core before any extractor runs
    #[must_use]
    fn describe(description: RouteDescription) -> RouteDescription {
        description
    }

    // gathered when the route is added: the state the app checks for when it starts, and what each request sets up
    fn require(_: &mut Requirements) {}

    fn from_request(
        request: &Request,
        arguments: &Arguments,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

// what an extractor rejects with is a response of the route too
pub(crate) fn describe_extractor<Extractor: FromRequest>(
    description: RouteDescription,
) -> RouteDescription {
    describe_responses::<Extractor::Rejection>(Extractor::describe(description))
}
