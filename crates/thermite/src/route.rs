use std::fmt;

use http::Method;
use thermite_core::{RouteDescription, RouteError};

use crate::handler::{Handler, HandlerEndpoint};
use crate::lifespan::Lifespan;
use crate::requirements::Requirements;
use crate::websocket::{WebSocketEndpoint, WebSocketHandler};

type AddRoute = Box<
    dyn FnOnce(&mut thermite_core::Router, RouteDescription) -> Result<(), RouteError>
        + Send
        + Sync,
>;

pub struct Route {
    kind: Kind,
    description: RouteDescription,
    // outermost router's first, as in FastAPI
    tags: Vec<String>,
    requirements: Requirements,
    add: AddRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Http,
    WebSocket,
}

impl Route {
    pub fn new<Function, Extractors>(
        method: Method,
        path: impl Into<String>,
        handler: Function,
    ) -> Self
    where
        Function: Handler<Extractors>,
        Extractors: 'static,
    {
        let mut requirements = Requirements::default();
        <Function as Handler<Extractors>>::require(&mut requirements);
        let scope = requirements.request_scope();
        Self {
            kind: Kind::Http,
            description: handler.describe(RouteDescription::new(method, path)),
            tags: Vec::new(),
            requirements,
            add: Box::new(move |router, description| {
                router.route(
                    description,
                    HandlerEndpoint::<_, Extractors>::new(handler, scope),
                )
            }),
        }
    }

    // a handshake is a GET, which only the WebSocket routes answer
    pub fn websocket<Function, Extractors>(path: impl Into<String>, handler: Function) -> Self
    where
        Function: WebSocketHandler<Extractors>,
        Extractors: 'static,
    {
        let mut requirements = Requirements::default();
        <Function as WebSocketHandler<Extractors>>::require(&mut requirements);
        let scope = requirements.request_scope();
        Self {
            kind: Kind::WebSocket,
            description: handler.describe(RouteDescription::new(Method::GET, path)),
            tags: Vec::new(),
            requirements,
            add: Box::new(move |router, description| {
                router.websocket(
                    description,
                    WebSocketEndpoint::<_, Extractors>::new(handler, scope),
                )
            }),
        }
    }

    pub(crate) fn nest_in(mut self, prefix: &str, tags: &[String]) -> Self {
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
        let needed_by = shown(self.kind, &description);
        (self.add)(router, description)?;
        lifespan.require(needed_by, self.requirements);
        Ok(())
    }
}

impl fmt::Debug for Route {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&shown(self.kind, &self.description))
    }
}

fn shown(kind: Kind, description: &RouteDescription) -> String {
    match kind {
        Kind::Http => format!("{} {}", description.method(), description.path()),
        Kind::WebSocket => format!("WEBSOCKET {}", description.path()),
    }
}

// `#[get]` and the other attributes add a type named after the function, holding its route
#[macro_export]
macro_rules! routes {
    ($($($segment:ident)::+),* $(,)?) => {
        [$($($segment)::+::route()),*]
    };
}
