use http::Method;
use thermite_core::{RouteError, StaticFiles};

use crate::handler::Handler;
use crate::method_routes::method_routes;
use crate::route::Route;
use crate::websocket::WebSocketHandler;

// a route can read path parameters from the prefixes around it, so routes are checked once included in the app
#[derive(Debug, Default)]
pub struct Router {
    prefix: String,
    tags: Vec<String>,
    routes: Vec<Route>,
    mounts: Vec<Mount>,
}

// checked as it is nested, since the joined path could no longer tell a missing slash from a prefix
#[derive(Debug)]
pub(crate) struct Mount {
    pub(crate) path: String,
    pub(crate) files: StaticFiles,
    pub(crate) invalid: Option<RouteError>,
}

impl Mount {
    fn nest_in(mut self, prefix: &str) -> Self {
        if prefix.is_empty() || self.invalid.is_some() {
            return self;
        }
        if !self.path.starts_with('/') {
            self.invalid = Some(RouteError::InvalidMountPath {
                path: self.path.clone(),
            });
            return self;
        }
        if !prefix.starts_with('/') || prefix.ends_with('/') {
            self.invalid = Some(RouteError::InvalidPrefix {
                prefix: prefix.to_owned(),
                path: self.path.clone(),
            });
        }
        self.path.insert_str(0, prefix);
        self
    }
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
    pub fn websocket<Function, Extractors>(
        mut self,
        path: impl Into<String>,
        handler: Function,
    ) -> Self
    where
        Function: WebSocketHandler<Extractors>,
        Extractors: 'static,
    {
        self.routes.push(Route::websocket(path, handler));
        self
    }

    #[must_use]
    pub fn routes(mut self, routes: impl IntoIterator<Item = Route>) -> Self {
        self.routes.extend(routes);
        self
    }

    #[must_use]
    pub fn mount(mut self, path: impl Into<String>, files: StaticFiles) -> Self {
        self.mounts.push(Mount {
            path: path.into(),
            files,
            invalid: None,
        });
        self
    }

    #[must_use]
    pub fn include_router(mut self, router: Router) -> Self {
        let (included, mounts) = router.into_parts();
        self.mounts.extend(mounts);
        self.routes(included)
    }

    pub(crate) fn into_parts(self) -> (impl Iterator<Item = Route>, Vec<Mount>) {
        let Self {
            prefix,
            tags,
            routes,
            mounts,
        } = self;
        let mounts = mounts
            .into_iter()
            .map(|mount| mount.nest_in(&prefix))
            .collect();
        let routes = routes
            .into_iter()
            .map(move |route| route.nest_in(&prefix, &tags));
        (routes, mounts)
    }
}
