use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;

pub type Request = http::Request<Bytes>;
pub type Response = http::Response<Bytes>;

pub trait RequestHandler: Send + Sync + 'static {
    fn call(&self, request: Request) -> impl Future<Output = Response> + Send;
}

impl<Function, Output> RequestHandler for Function
where
    Function: Fn(Request) -> Output + Send + Sync + 'static,
    Output: Future<Output = Response> + Send,
{
    fn call(&self, request: Request) -> impl Future<Output = Response> + Send {
        self(request)
    }
}

pub(crate) type BoxedResponseFuture<'handler> =
    Pin<Box<dyn Future<Output = Response> + Send + 'handler>>;

// RequestHandler returns impl Future, which rules out dyn RequestHandler, so storage goes through this
pub(crate) trait Endpoint: Send + Sync + 'static {
    fn call_boxed(&self, request: Request) -> BoxedResponseFuture<'_>;
}

impl<Handler: RequestHandler> Endpoint for Handler {
    fn call_boxed(&self, request: Request) -> BoxedResponseFuture<'_> {
        Box::pin(self.call(request))
    }
}
