use std::convert::Infallible;
use std::future::{Future, ready};

use thermite_core::{Arguments, BackgroundTasks, Request};

use crate::extract::FromRequest;

// the handler puts one list in each request, so its dependencies add to the same tasks
impl FromRequest for BackgroundTasks {
    type Rejection = Infallible;

    fn from_request(
        request: &Request,
        _: &Arguments,
    ) -> impl Future<Output = Result<Self, Infallible>> + Send {
        ready(Ok(request
            .extensions()
            .get::<Self>()
            .cloned()
            .unwrap_or_default()))
    }
}
