use std::future::Future;

use thermite_core::{Arguments, Request, Response, ResponseDescription, RouteDescription};

use super::Handler;
use crate::state::RequiredState;

#[derive(Debug, Clone)]
pub struct WithResponse<Function> {
    handler: Function,
    response: ResponseDescription,
}

impl<Function> WithResponse<Function> {
    pub(super) fn new(handler: Function, response: ResponseDescription) -> Self {
        Self { handler, response }
    }
}

// a function that is not a handler would otherwise be pointed at this impl
#[diagnostic::do_not_recommend]
impl<Function, Extractors> Handler<Extractors> for WithResponse<Function>
where
    Function: Handler<Extractors>,
{
    // after what the wrapped handler describes, so chained responses keep the order they were written in
    fn describe(&self, description: RouteDescription) -> RouteDescription {
        self.handler
            .describe(description)
            .response(self.response.clone())
    }

    fn require_state(required: &mut RequiredState) {
        Function::require_state(required);
    }

    fn call(
        &self,
        request: Request,
        arguments: Arguments,
    ) -> impl Future<Output = Response> + Send {
        self.handler.call(request, arguments)
    }
}
