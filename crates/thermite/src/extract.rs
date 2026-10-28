use std::future::Future;

use thermite_core::{Arguments, Request, Response, RouteDescription};

pub trait FromRequest: Sized + Send {
    type Rejection: Into<Response>;

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
