use std::error::Error;
use std::future::Future;

use http::Method;
use thermite_core::{Docs, OpenApi, Request, RequestHandler, Response, RouteError};
use tokio::sync::OnceCell;

use crate::handler::Handler;
use crate::lifespan::Lifespan;
use crate::method_routes::method_routes;
use crate::resource::Resource;
use crate::router::{PendingRoute, Router};

#[derive(Debug, Default)]
pub struct App {
    router: thermite_core::Router,
    lifespan: Lifespan,
    openapi: OpenApi,
    // built when the app starts, once every route is added
    docs: OnceCell<Docs>,
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
        PendingRoute::new(method, path.into(), handler)
            .add_to(&mut self.router, &mut self.lifespan)?;
        Ok(self)
    }

    method_routes!(Result<Self, RouteError>);

    pub fn include_router(mut self, router: Router) -> Result<Self, RouteError> {
        for route in router.into_routes() {
            route.add_to(&mut self.router, &mut self.lifespan)?;
        }
        Ok(self)
    }
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
