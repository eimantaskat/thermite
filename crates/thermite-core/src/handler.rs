use std::error::Error;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use crate::arguments::Arguments;

pub type Request = http::Request<Bytes>;
pub type Response = http::Response<Bytes>;

// the server holds the handler in an Arc, which a 'static future such as a middleware's Next can keep
pub trait RequestHandler: Send + Sync + 'static {
    fn call(self: &Arc<Self>, request: Request) -> impl Future<Output = Response> + Send + 'static;

    // the server awaits this before accepting connections, and serves nothing if it fails
    fn startup(&self) -> impl Future<Output = Result<(), Box<dyn Error + Send + Sync>>> + Send {
        ready(Ok(()))
    }

    // the server awaits this once every connection is closed, and only after a successful startup
    fn shutdown(&self) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

impl<Function, Output> RequestHandler for Function
where
    Function: Fn(Request) -> Output + Send + Sync + 'static,
    Output: Future<Output = Response> + Send + 'static,
{
    fn call(self: &Arc<Self>, request: Request) -> impl Future<Output = Response> + Send + 'static {
        self(request)
    }
}

pub trait RouteHandler: Send + Sync + 'static {
    fn call(&self, request: Request, arguments: Arguments)
    -> impl Future<Output = Response> + Send;
}

impl<Function, Output> RouteHandler for Function
where
    Function: Fn(Request, Arguments) -> Output + Send + Sync + 'static,
    Output: Future<Output = Response> + Send,
{
    fn call(
        &self,
        request: Request,
        arguments: Arguments,
    ) -> impl Future<Output = Response> + Send {
        self(request, arguments)
    }
}

pub(crate) type BoxedResponseFuture<'handler> =
    Pin<Box<dyn Future<Output = Response> + Send + 'handler>>;

// RouteHandler returns impl Future, which rules out dyn RouteHandler, so storage goes through this
pub(crate) trait Endpoint: Send + Sync + 'static {
    fn call_boxed(&self, request: Request, arguments: Arguments) -> BoxedResponseFuture<'_>;
}

impl<Handler: RouteHandler> Endpoint for Handler {
    fn call_boxed(&self, request: Request, arguments: Arguments) -> BoxedResponseFuture<'_> {
        Box::pin(self.call(request, arguments))
    }
}
