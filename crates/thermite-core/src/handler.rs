use std::future::Future;

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
