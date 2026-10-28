use std::error::Error;
use std::future::Future;

use http::Method;
use thermite_core::{Request, RequestHandler, Response, RouteDescription, RouteError, Router};

use crate::handler::{Handler, HandlerEndpoint};
use crate::lifespan::Lifespan;
use crate::resource::Resource;
use crate::state::RequiredState;

#[derive(Debug, Default)]
pub struct App {
    router: Router,
    lifespan: Lifespan,
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

    #[must_use]
    pub fn state<T: Send + Sync + 'static>(mut self, value: T) -> Self {
        self.lifespan.add_state(value);
        self
    }

    // resources start in the order added, so each can need the ones before it
    #[must_use]
    pub fn resource<T: Resource>(mut self) -> Self {
        self.lifespan.add_resource::<T>();
        self
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
        let path = path.into();
        let needed_by = format!("{method} {path}");
        let description =
            <Function as Handler<Extractors>>::describe(RouteDescription::new(method, path));
        let router = self
            .router
            .route(description, HandlerEndpoint::<_, Extractors>::new(handler))?;
        let mut lifespan = self.lifespan;
        let mut required = RequiredState::default();
        <Function as Handler<Extractors>>::require_state(&mut required);
        lifespan.require(needed_by, required);
        Ok(Self { router, lifespan })
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
    fn call(&self, mut request: Request) -> impl Future<Output = Response> + Send {
        self.lifespan.hand_state_to(&mut request);
        self.router.call(request)
    }

    async fn startup(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(self.lifespan.startup().await?)
    }

    fn shutdown(&self) -> impl Future<Output = ()> + Send {
        self.lifespan.shutdown()
    }
}
