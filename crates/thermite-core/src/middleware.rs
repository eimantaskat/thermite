use std::fmt;
use std::future::Future;
use std::sync::Arc;

use crate::handler::{BoxedResponseFuture, Request, Response};

pub trait Middleware: Send + Sync + 'static {
    fn call(&self, request: Request, next: Next) -> impl Future<Output = Response> + Send;
}

impl<Function, Output> Middleware for Function
where
    Function: Fn(Request, Next) -> Output + Send + Sync + 'static,
    Output: Future<Output = Response> + Send,
{
    fn call(&self, request: Request, next: Next) -> impl Future<Output = Response> + Send {
        self(request, next)
    }
}

// like FastAPI, the middleware added last runs first
#[derive(Default)]
pub struct MiddlewareStack(Vec<Box<dyn BoxedMiddleware>>);

impl MiddlewareStack {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, middleware: impl Middleware) {
        self.0.push(Box::new(middleware));
    }
}

impl fmt::Debug for MiddlewareStack {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MiddlewareStack")
            .field("len", &self.0.len())
            .finish()
    }
}

// what a front-end's app implements: its middleware, and the endpoint they lead to
pub trait Pipeline: Send + Sync + 'static {
    fn middleware_stack(&self) -> &MiddlewareStack;

    fn endpoint(&self, request: Request) -> impl Future<Output = Response> + Send;
}

// owns the pipeline, so a middleware can keep it, move it to another task or hand it to Python
pub struct Next {
    pipeline: Arc<dyn BoxedPipeline>,
    remaining: usize,
}

impl Next {
    pub fn start(
        pipeline: &Arc<impl Pipeline>,
        request: Request,
    ) -> impl Future<Output = Response> + Send + 'static {
        let remaining = pipeline.middleware_stack().0.len();
        let next = Self {
            pipeline: Arc::clone(pipeline) as Arc<dyn BoxedPipeline>,
            remaining,
        };
        next.run(request)
    }

    pub async fn run(mut self, request: Request) -> Response {
        let pipeline = Arc::clone(&self.pipeline);
        let index = self.remaining.checked_sub(1);
        match index.and_then(|index| pipeline.middleware_stack().0.get(index)) {
            Some(middleware) => {
                self.remaining -= 1;
                middleware.call_boxed(request, self).await
            }
            None => pipeline.endpoint_boxed(request).await,
        }
    }
}

impl fmt::Debug for Next {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Next")
            .field("remaining", &self.remaining)
            .finish_non_exhaustive()
    }
}

// Middleware and Pipeline return impl Future, which rules out dyn, so storage goes through these
trait BoxedMiddleware: Send + Sync + 'static {
    fn call_boxed(&self, request: Request, next: Next) -> BoxedResponseFuture<'_>;
}

impl<Inner: Middleware> BoxedMiddleware for Inner {
    fn call_boxed(&self, request: Request, next: Next) -> BoxedResponseFuture<'_> {
        Box::pin(self.call(request, next))
    }
}

trait BoxedPipeline: Send + Sync + 'static {
    fn middleware_stack(&self) -> &MiddlewareStack;

    fn endpoint_boxed(&self, request: Request) -> BoxedResponseFuture<'_>;
}

impl<Inner: Pipeline> BoxedPipeline for Inner {
    fn middleware_stack(&self) -> &MiddlewareStack {
        Pipeline::middleware_stack(self)
    }

    fn endpoint_boxed(&self, request: Request) -> BoxedResponseFuture<'_> {
        Box::pin(self.endpoint(request))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::future::ready;
    use std::sync::Mutex;

    use bytes::Bytes;
    use http::StatusCode;

    use super::*;

    struct Recording {
        middleware: MiddlewareStack,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl Recording {
        fn new() -> Self {
            Self {
                middleware: MiddlewareStack::new(),
                events: Arc::default(),
            }
        }

        fn record(events: &Mutex<Vec<String>>, event: impl Into<String>) {
            events.lock().unwrap().push(event.into());
        }

        fn wrap(&mut self, name: &'static str) {
            let events = Arc::clone(&self.events);
            self.middleware.push(move |request: Request, next: Next| {
                let events = Arc::clone(&events);
                async move {
                    Self::record(&events, format!("{name} before"));
                    let response = next.run(request).await;
                    Self::record(&events, format!("{name} after"));
                    response
                }
            });
        }
    }

    impl Pipeline for Recording {
        fn middleware_stack(&self) -> &MiddlewareStack {
            &self.middleware
        }

        fn endpoint(&self, request: Request) -> impl Future<Output = Response> + Send {
            Self::record(&self.events, "endpoint");
            ready(Response::new(Bytes::from(request.uri().path().to_owned())))
        }
    }

    fn request(path: &str) -> Request {
        http::Request::builder()
            .uri(path)
            .body(Bytes::new())
            .unwrap()
    }

    #[tokio::test]
    async fn runs_the_middleware_added_last_first() {
        let mut pipeline = Recording::new();
        pipeline.wrap("inner");
        pipeline.wrap("outer");
        let pipeline = Arc::new(pipeline);

        let response = Next::start(&pipeline, request("/items")).await;

        assert_eq!(response.body(), "/items");
        assert_eq!(
            *pipeline.events.lock().unwrap(),
            [
                "outer before",
                "inner before",
                "endpoint",
                "inner after",
                "outer after"
            ]
        );
    }

    #[tokio::test]
    async fn a_middleware_can_answer_without_calling_next() {
        let mut pipeline = Recording::new();
        pipeline.wrap("inner");
        pipeline
            .middleware
            .push(|_request: Request, _next: Next| async {
                let mut response = Response::default();
                *response.status_mut() = StatusCode::UNAUTHORIZED;
                response
            });
        let pipeline = Arc::new(pipeline);

        let response = Next::start(&pipeline, request("/items")).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(pipeline.events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn next_can_run_on_another_task_and_change_the_request() {
        let mut pipeline = Recording::new();
        pipeline
            .middleware
            .push(|request: Request, next: Next| async move {
                let (mut parts, body) = request.into_parts();
                parts.uri = "/rewritten".parse().unwrap();
                let request = Request::from_parts(parts, body);
                tokio::spawn(next.run(request)).await.unwrap()
            });
        let pipeline = Arc::new(pipeline);

        let response = Next::start(&pipeline, request("/items")).await;

        assert_eq!(response.body(), "/rewritten");
    }

    #[tokio::test]
    async fn debug_output_counts_the_middleware_and_what_is_left_to_run() {
        let mut pipeline = Recording::new();
        pipeline.wrap("inner");
        pipeline
            .middleware
            .push(|_request: Request, next: Next| async move {
                Response::new(Bytes::from(format!("{next:?}")))
            });
        let shown_stack = format!("{:?}", pipeline.middleware);

        let response = Next::start(&Arc::new(pipeline), request("/")).await;

        assert_eq!(shown_stack, "MiddlewareStack { len: 2 }");
        assert_eq!(response.body(), "Next { remaining: 1, .. }");
    }
}
