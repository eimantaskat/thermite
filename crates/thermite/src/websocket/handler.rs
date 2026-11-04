use std::future::Future;
use std::marker::PhantomData;

use thermite_core::{Arguments, Request, RouteDescription, WebSocketDisconnect};

use super::WebSocket;
use crate::arity::for_each_arity;
use crate::extract::FromRequest;
use crate::handler::set_up;
use crate::requirements::{RequestScope, Requirements};

// a disconnect that ends the handler through `?` is how a WebSocket usually ends, as in Starlette
pub trait WebSocketOutput {}

impl WebSocketOutput for () {}

impl WebSocketOutput for Result<(), WebSocketDisconnect> {}

#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot handle a WebSocket route",
    label = "not a WebSocket handler",
    note = "a WebSocket handler is an async fn taking a `WebSocket` first, then arguments that implement `FromRequest`, and returning `()` or `Result<(), WebSocketDisconnect>`"
)]
pub trait WebSocketHandler<Extractors>: Send + Sync + 'static {
    fn describe(&self, description: RouteDescription) -> RouteDescription;

    fn require(requirements: &mut Requirements);

    fn call(
        &self,
        request: Request,
        arguments: Arguments,
        socket: thermite_core::WebSocket,
    ) -> impl Future<Output = ()> + Send;
}

macro_rules! impl_websocket_handler {
    ($($extractor:ident),*) => {
        impl<Function, Output, $($extractor),*> WebSocketHandler<($($extractor,)*)> for Function
        where
            Function: Fn(WebSocket, $($extractor),*) -> Output + Send + Sync + 'static,
            Output: Future<Output: WebSocketOutput> + Send,
            $($extractor: FromRequest,)*
        {
            fn describe(&self, description: RouteDescription) -> RouteDescription {
                <($($extractor,)*) as FromRequest>::describe(description)
            }

            fn require(requirements: &mut Requirements) {
                <($($extractor,)*) as FromRequest>::require(requirements);
            }

            // each value is named after its type parameter
            #[allow(non_snake_case)]
            fn call(
                &self,
                request: Request,
                arguments: Arguments,
                socket: thermite_core::WebSocket,
            ) -> impl Future<Output = ()> + Send {
                async move {
                    match <($($extractor,)*) as FromRequest>::from_request(&request, &arguments).await {
                        Ok(($($extractor,)*)) => {
                            self(WebSocket(socket), $($extractor),*).await;
                        }
                        // like an HTTPException raised by a dependency before accepting
                        Err(rejection) => socket.deny(rejection),
                    }
                }
            }
        }
    };
}

for_each_arity!(impl_websocket_handler);

// a blanket core WebSocketHandler impl over every WebSocketHandler<Extractors> would leave Extractors unconstrained
pub(crate) struct WebSocketEndpoint<Function, Extractors> {
    handler: Function,
    scope: RequestScope,
    extractors: PhantomData<fn() -> Extractors>,
}

impl<Function, Extractors> WebSocketEndpoint<Function, Extractors> {
    pub(crate) fn new(handler: Function, scope: RequestScope) -> Self {
        Self {
            handler,
            scope,
            extractors: PhantomData,
        }
    }
}

impl<Function, Extractors> thermite_core::WebSocketHandler
    for WebSocketEndpoint<Function, Extractors>
where
    Function: WebSocketHandler<Extractors>,
    Extractors: 'static,
{
    fn call(
        &self,
        mut request: Request,
        arguments: Arguments,
        socket: thermite_core::WebSocket,
    ) -> impl Future<Output = ()> + Send {
        set_up(self.scope, &mut request);
        self.handler.call(request, arguments, socket)
    }
}
