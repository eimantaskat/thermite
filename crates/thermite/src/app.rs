use std::error::Error;
use std::future::Future;

use http::Method;
use thermite_core::{
    Docs, OpenApi, Request, RequestHandler, Response, RouteDescription, RouteError, Router,
};
use tokio::sync::OnceCell;

use crate::handler::{Handler, HandlerEndpoint, handler_name};
use crate::lifespan::Lifespan;
use crate::resource::Resource;
use crate::state::RequiredState;

#[derive(Debug, Default)]
pub struct App {
    router: Router,
    lifespan: Lifespan,
    openapi: OpenApi,
    // built when the app starts, once every route is added
    docs: OnceCell<Docs>,
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

    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.openapi = self.openapi.title(title);
        self
    }

    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.openapi = self.openapi.version(version);
        self
    }

    #[must_use]
    pub fn openapi_path(mut self, path: impl Into<String>) -> Self {
        self.openapi = self.openapi.openapi_path(path);
        self
    }

    #[must_use]
    pub fn docs_path(mut self, path: impl Into<String>) -> Self {
        self.openapi = self.openapi.docs_path(path);
        self
    }

    #[must_use]
    pub fn without_openapi(mut self) -> Self {
        self.openapi = self.openapi.without_openapi();
        self
    }

    #[must_use]
    pub fn without_docs(mut self) -> Self {
        self.openapi = self.openapi.without_docs();
        self
    }

    // resources start in the order added, so each can need the ones before it
    #[must_use]
    pub fn resource<T: Resource>(mut self) -> Self {
        self.lifespan.add_resource::<T>();
        self
    }

    pub fn route<Function, Extractors>(
        mut self,
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
        let mut description = RouteDescription::new(method, path);
        if let Some(name) = handler_name::<Function>() {
            description = description.with_name(name);
        }
        let description = <Function as Handler<Extractors>>::describe(description);
        self.router = self
            .router
            .route(description, HandlerEndpoint::<_, Extractors>::new(handler))?;
        let mut required = RequiredState::default();
        <Function as Handler<Extractors>>::require_state(&mut required);
        self.lifespan.require(needed_by, required);
        Ok(self)
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
    async fn call(&self, mut request: Request) -> Response {
        if let Some(response) = self.docs.get().and_then(|docs| docs.respond(&request)) {
            return response;
        }
        self.lifespan.hand_state_to(&mut request);
        self.router.call(request).await
    }

    // the docs first, since they start nothing that would need stopping when they fail
    async fn startup(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.docs
            .get_or_try_init(|| async { Docs::new(&self.openapi, &self.router) })
            .await?;
        Ok(self.lifespan.startup().await?)
    }

    fn shutdown(&self) -> impl Future<Output = ()> + Send {
        self.lifespan.shutdown()
    }
}
