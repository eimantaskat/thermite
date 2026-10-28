use std::future::Future;

use http::Method;
use thermite_core::{Request, RequestHandler, Response, RouteDescription, RouteError, Router};

use crate::handler::{Handler, HandlerEndpoint};

#[derive(Debug, Default)]
pub struct App {
    router: Router,
}

macro_rules! method_routes {
    ($($name:ident => $method:ident),*) => {
        $(
            pub fn $name<Function, Extractors>(
                self,
                path: impl Into<String>,
                handler: Function,
            ) -> Result<Self, RouteError>
            where
                Function: Handler<Extractors>,
                Extractors: 'static,
            {
                self.route(Method::$method, path, handler)
            }
        )*
    };
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn route<Function, Extractors>(
        self,
        method: Method,
        path: impl Into<String>,
        handler: Function,
    ) -> Result<Self, RouteError>
    where
        Function: Handler<Extractors>,
        Extractors: 'static,
    {
        let description =
            <Function as Handler<Extractors>>::describe(RouteDescription::new(method, path));
        let router = self
            .router
            .route(description, HandlerEndpoint::<_, Extractors>::new(handler))?;
        Ok(Self { router })
    }

    method_routes!(
        get => GET,
        post => POST,
        put => PUT,
        patch => PATCH,
        delete => DELETE,
        head => HEAD,
        options => OPTIONS,
        trace => TRACE
    );
}

impl RequestHandler for App {
    fn call(&self, request: Request) -> impl Future<Output = Response> + Send {
        self.router.call(request)
    }
}
