use std::convert::Infallible;
use std::future::{Future, ready};

use thermite_core::{Arguments, BackgroundTasks, Request};

use crate::extract::FromRequest;
use crate::requirements::Requirements;

// a route taking them puts one list in each request, so its dependencies add to the same tasks
impl FromRequest for BackgroundTasks {
    type Rejection = Infallible;

    fn require(requirements: &mut Requirements) {
        requirements.collect_background_tasks();
    }

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
