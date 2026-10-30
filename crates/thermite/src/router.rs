use std::fmt;

use http::Method;
use thermite_core::{RouteDescription, RouteError};

use crate::handler::{Handler, HandlerEndpoint, handler_name};
use crate::lifespan::Lifespan;
use crate::method_routes::method_routes;
use crate::state::RequiredState;

// a route can read path parameters from the prefixes around it, so routes are checked once included in the app
#[derive(Debug, Default)]
pub struct Router {
    prefix: String,
    tags: Vec<String>,
    routes: Vec<PendingRoute>,
}

impl Router {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    #[must_use]
    pub fn route<Function, Extractors>(
        mut self,
        method: Method,
        path: impl Into<String>,
        handler: Function,
    ) -> Self
    where
        Function: Handler<Extractors>,
        Extractors: 'static,
    {
        self.routes
            .push(PendingRoute::new(method, path.into(), handler));
        self
    }

    method_routes!(
        #[must_use]
        Self
    );

    #[must_use]
    pub fn include_router(mut self, router: Router) -> Self {
        self.routes.extend(router.into_routes());
        self
    }

    pub(crate) fn into_routes(self) -> impl Iterator<Item = PendingRoute> {
        let Self {
            prefix,
            tags,
            routes,
        } = self;
        routes
            .into_iter()
            .map(move |route| route.nest_in(&prefix, &tags))
    }
}

type AddRoute = Box<
    dyn FnOnce(&mut thermite_core::Router, RouteDescription) -> Result<(), RouteError>
        + Send
        + Sync,
>;

pub(crate) struct PendingRoute {
    description: RouteDescription,
    // outermost router's first, as in FastAPI
    tags: Vec<String>,
    required: RequiredState,
    add: AddRoute,
}

impl PendingRoute {
    pub(crate) fn new<Function, Extractors>(method: Method, path: String, handler: Function) -> Self
    where
        Function: Handler<Extractors>,
        Extractors: 'static,
    {
        let mut description = RouteDescription::new(method, path);
        if let Some(name) = handler_name::<Function>() {
            description = description.with_name(name);
        }
        let mut required = RequiredState::default();
        <Function as Handler<Extractors>>::require_state(&mut required);
        Self {
            description: <Function as Handler<Extractors>>::describe(description),
            tags: Vec::new(),
            required,
            add: Box::new(move |router, description| {
                router.route(description, HandlerEndpoint::<_, Extractors>::new(handler))
            }),
        }
    }

    fn nest_in(mut self, prefix: &str, tags: &[String]) -> Self {
        self.description = self.description.with_prefix(prefix);
        self.tags.splice(0..0, tags.iter().cloned());
        self
    }

    pub(crate) fn add_to(
        self,
        router: &mut thermite_core::Router,
        lifespan: &mut Lifespan,
    ) -> Result<(), RouteError> {
        let description = self
            .tags
            .into_iter()
            .fold(self.description, RouteDescription::with_tag);
        let needed_by = format!("{} {}", description.method(), description.path());
        (self.add)(router, description)?;
        lifespan.require(needed_by, self.required);
        Ok(())
    }
}

impl fmt::Debug for PendingRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {}",
            self.description.method(),
            self.description.path()
        )
    }
}
