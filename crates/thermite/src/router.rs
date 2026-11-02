use http::Method;

use crate::handler::Handler;
use crate::method_routes::method_routes;
use crate::route::Route;

// a route can read path parameters from the prefixes around it, so routes are checked once included in the app
#[derive(Debug, Default)]
pub struct Router {
    prefix: String,
    tags: Vec<String>,
    routes: Vec<Route>,
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
        self.routes.push(Route::new(method, path, handler));
        self
    }

    method_routes!(
        #[must_use]
        Self
    );

    #[must_use]
    pub fn routes(mut self, routes: impl IntoIterator<Item = Route>) -> Self {
        self.routes.extend(routes);
        self
    }

    #[must_use]
    pub fn include_router(self, router: Router) -> Self {
        self.routes(router.into_routes())
    }

    pub(crate) fn into_routes(self) -> impl Iterator<Item = Route> {
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
