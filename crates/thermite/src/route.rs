use std::fmt;

use http::Method;
use thermite_core::{RouteDescription, RouteError};

use crate::handler::{Handler, HandlerEndpoint};
use crate::lifespan::Lifespan;
use crate::state::RequiredState;

type AddRoute = Box<
    dyn FnOnce(&mut thermite_core::Router, RouteDescription) -> Result<(), RouteError>
        + Send
        + Sync,
>;

pub struct Route {
    description: RouteDescription,
    // outermost router's first, as in FastAPI
    tags: Vec<String>,
    required: RequiredState,
    add: AddRoute,
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
        let mut required = RequiredState::default();
        <Function as Handler<Extractors>>::require_state(&mut required);
        Self {
            description: handler.describe(RouteDescription::new(method, path)),
            tags: Vec::new(),
            required,
            add: Box::new(move |router, description| {
                router.route(description, HandlerEndpoint::<_, Extractors>::new(handler))
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
        let needed_by = format!("{} {}", description.method(), description.path());
        (self.add)(router, description)?;
        lifespan.require(needed_by, self.required);
        Ok(())
    }
}

impl fmt::Debug for Route {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {}",
            self.description.method(),
            self.description.path()
        )
    }
}

// `#[get]` and the other attributes add a type named after the function, holding its route
#[macro_export]
macro_rules! routes {
    ($($($segment:ident)::+),* $(,)?) => {
        [$($($segment)::+::route()),*]
    };
}
