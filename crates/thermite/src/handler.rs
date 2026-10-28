use std::future::Future;
use std::marker::PhantomData;

use thermite_core::{Arguments, Request, Response, RouteDescription, RouteHandler};

use crate::extract::FromRequest;

// `Extractors` is the tuple of argument types, so one function type can only match one impl
pub trait Handler<Extractors>: Send + Sync + 'static {
    fn describe(description: RouteDescription) -> RouteDescription;

    fn call(&self, request: Request, arguments: Arguments)
    -> impl Future<Output = Response> + Send;
}

macro_rules! impl_handler {
    ($($extractor:ident),*) => {
        impl<Function, Output, $($extractor),*> Handler<($($extractor,)*)> for Function
        where
            Function: Fn($($extractor),*) -> Output + Send + Sync + 'static,
            Output: Future<Output = Response> + Send,
            $($extractor: FromRequest,)*
        {
            fn describe(description: RouteDescription) -> RouteDescription {
                $(let description = $extractor::describe(description);)*
                description
            }

            // each value is named after its type parameter, and a handler without extractors reads nothing
            #[allow(non_snake_case, unused_variables)]
            fn call(
                &self,
                request: Request,
                arguments: Arguments,
            ) -> impl Future<Output = Response> + Send {
                async move {
                    $(
                        let $extractor = match $extractor::from_request(&request, &arguments).await {
                            Ok(value) => value,
                            Err(rejection) => return rejection.into(),
                        };
                    )*
                    self($($extractor),*).await
                }
            }
        }
    };
}

// calls `impl_handler!` once per prefix of the list: no extractors, then one, up to all sixteen
macro_rules! impl_handler_for_each_arity {
    ([$($done:ident),*]) => {
        impl_handler!($($done),*);
    };
    ([$($done:ident),*] $next:ident $($rest:ident)*) => {
        impl_handler!($($done),*);
        impl_handler_for_each_arity!([$($done,)* $next] $($rest)*);
    };
}

impl_handler_for_each_arity!([] Extractor1 Extractor2 Extractor3 Extractor4 Extractor5 Extractor6
    Extractor7 Extractor8 Extractor9 Extractor10 Extractor11 Extractor12 Extractor13 Extractor14
    Extractor15 Extractor16);

// a blanket RouteHandler impl over every Handler<Extractors> would leave Extractors unconstrained
pub(crate) struct HandlerEndpoint<Function, Extractors> {
    handler: Function,
    // `fn() -> Extractors` keeps the endpoint Send and Sync whatever the extractor types are
    extractors: PhantomData<fn() -> Extractors>,
}

impl<Function, Extractors> HandlerEndpoint<Function, Extractors> {
    pub(crate) fn new(handler: Function) -> Self {
        Self {
            handler,
            extractors: PhantomData,
        }
    }
}

impl<Function, Extractors> RouteHandler for HandlerEndpoint<Function, Extractors>
where
    Function: Handler<Extractors>,
    Extractors: 'static,
{
    fn call(
        &self,
        request: Request,
        arguments: Arguments,
    ) -> impl Future<Output = Response> + Send {
        self.handler.call(request, arguments)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::future::ready;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use http::header::AUTHORIZATION;
    use http::{Method, StatusCode};
    use thermite_core::{Parameter, Schema};

    use super::*;

    fn status(code: StatusCode) -> Response {
        let mut response = Response::default();
        *response.status_mut() = code;
        response
    }

    struct Unauthorized;

    impl From<Unauthorized> for Response {
        fn from(_: Unauthorized) -> Self {
            status(StatusCode::UNAUTHORIZED)
        }
    }

    struct Token;

    impl FromRequest for Token {
        type Rejection = Unauthorized;

        fn from_request(
            request: &Request,
            _: &Arguments,
        ) -> impl Future<Output = Result<Self, Unauthorized>> + Send {
            let authorized = request.headers().contains_key(AUTHORIZATION);
            ready(if authorized {
                Ok(Self)
            } else {
                Err(Unauthorized)
            })
        }
    }

    #[derive(Clone, Default)]
    struct Runs(Arc<AtomicUsize>);

    // counts its runs in the request's own counter, and suspends once like an extractor waiting on I/O
    struct Counted;

    impl FromRequest for Counted {
        type Rejection = Response;

        async fn from_request(request: &Request, _: &Arguments) -> Result<Self, Response> {
            if let Some(Runs(runs)) = request.extensions().get::<Runs>() {
                runs.fetch_add(1, Ordering::SeqCst);
            }
            tokio::task::yield_now().await;
            Ok(Self)
        }
    }

    struct Limit;

    impl FromRequest for Limit {
        type Rejection = Response;

        fn describe(description: RouteDescription) -> RouteDescription {
            description.parameter(Parameter::query("limit", Schema::integer()))
        }

        fn from_request(
            _: &Request,
            _: &Arguments,
        ) -> impl Future<Output = Result<Self, Response>> + Send {
            ready(Ok(Self))
        }
    }

    struct Offset;

    impl FromRequest for Offset {
        type Rejection = Response;

        fn describe(description: RouteDescription) -> RouteDescription {
            description.parameter(Parameter::query("offset", Schema::integer()).with_default(0))
        }

        fn from_request(
            _: &Request,
            _: &Arguments,
        ) -> impl Future<Output = Result<Self, Response>> + Send {
            ready(Ok(Self))
        }
    }

    fn describe<Function: Handler<Extractors>, Extractors>(_: &Function) -> RouteDescription {
        Function::describe(RouteDescription::new(Method::GET, "/items"))
    }

    #[test]
    fn describes_what_each_extractor_declares_in_argument_order() {
        let description = describe(&|_: Offset, _: Token, _: Limit| async { Response::default() });

        assert_eq!(
            description,
            RouteDescription::new(Method::GET, "/items")
                .parameter(Parameter::query("offset", Schema::integer()).with_default(0))
                .parameter(Parameter::query("limit", Schema::integer()))
        );
    }

    #[test]
    fn a_handler_without_extractors_declares_nothing() {
        let description = describe(&|| async { Response::default() });

        assert_eq!(description, RouteDescription::new(Method::GET, "/items"));
    }

    #[tokio::test]
    async fn calls_the_handler_once_every_extractor_succeeds() {
        let handler = |_: Token| async { status(StatusCode::OK) };
        let mut request = Request::default();
        request
            .headers_mut()
            .insert(AUTHORIZATION, "Bearer x".parse().unwrap());

        let response = Handler::call(&handler, request, Arguments::default()).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn answers_with_the_first_rejection_and_runs_nothing_after_it() {
        let handler_runs = Arc::new(AtomicUsize::new(0));
        let handler = {
            let handler_runs = Arc::clone(&handler_runs);
            move |_: Counted, _: Token, _: Counted| {
                handler_runs.fetch_add(1, Ordering::SeqCst);
                async { status(StatusCode::OK) }
            }
        };

        let extractor_runs = Runs::default();
        let mut request = Request::default();
        request.extensions_mut().insert(extractor_runs.clone());

        let response = Handler::call(&handler, request, Arguments::default()).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(extractor_runs.0.load(Ordering::SeqCst), 1);
        assert_eq!(handler_runs.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn accepts_handlers_with_up_to_sixteen_extractors() {
        #[allow(clippy::too_many_arguments)] // the point of the test is the argument count
        async fn sixteen(
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Limit,
            _: Offset,
        ) -> Response {
            status(StatusCode::OK)
        }

        let response = Handler::call(&sixteen, Request::default(), Arguments::default()).await;

        assert_eq!(response.status(), StatusCode::OK);
    }
}
