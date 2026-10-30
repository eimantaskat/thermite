mod check;
mod extract;
mod path;
mod table;

use std::future::Future;
use std::sync::Arc;

use http::header::ALLOW;
use http::{HeaderValue, Method, StatusCode};

use crate::arguments::Arguments;
use crate::description::{Parameter, RouteDescription};
use crate::handler::{Endpoint, Request, RequestHandler, Response, RouteHandler};
use crate::http_error::HttpError;
use crate::route_error::RouteError;
use crate::schema::Schema;
use crate::validation::validation_failed;
use check::{Inputs, check_description};
use extract::extract_arguments;
use path::PathPattern;
use table::{Lookup, RouteTable};

#[derive(Debug)]
pub struct Router {
    routes: RouteTable<Route>,
}

struct Route {
    // the table orders routes by how specific they are, while the docs list them as registered
    registered: usize,
    name: Option<String>,
    tags: Vec<String>,
    inputs: Inputs,
    endpoint: Box<dyn Endpoint>,
}

// what the docs show of a route
pub(crate) struct Operation<'router> {
    pub(crate) method: &'router Method,
    pub(crate) path: &'router str,
    pub(crate) name: Option<&'router str>,
    pub(crate) tags: &'router [String],
    pub(crate) parameters: &'router [Parameter],
    pub(crate) body: Option<&'router Schema>,
}

enum Dispatch<'router> {
    Endpoint(&'router dyn Endpoint, Arguments),
    // 404, 405 or 422, answered without calling a handler
    Respond(Response),
}

impl Default for Router {
    fn default() -> Self {
        Self {
            routes: RouteTable::new(),
        }
    }
}

impl Router {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // a route that fails to register leaves the router as it was
    pub fn route(
        &mut self,
        description: RouteDescription,
        handler: impl RouteHandler,
    ) -> Result<(), RouteError> {
        if let Some(prefix) = description.invalid_prefix() {
            return Err(RouteError::InvalidPrefix {
                prefix: prefix.to_owned(),
                path: description.path().to_owned(),
            });
        }
        let pattern = PathPattern::parse(description.path())?;
        let method = description.method().clone();
        let name = description.name().map(str::to_owned);
        let tags = description.tags().to_vec();
        let inputs = check_description(description, &pattern)?;
        let registered = self.routes.len();
        self.routes.insert(
            method,
            pattern,
            Route {
                registered,
                name,
                tags,
                inputs,
                endpoint: Box::new(handler),
            },
        )
    }

    pub(crate) fn operations(&self) -> Vec<Operation<'_>> {
        let mut routes: Vec<_> = self.routes.iter().collect();
        routes.sort_by_key(|(_, _, route)| route.registered);
        routes
            .into_iter()
            .map(|(method, pattern, route)| Operation {
                method,
                path: pattern.as_str(),
                name: route.name.as_deref(),
                tags: &route.tags,
                parameters: &route.inputs.parameters,
                body: route.inputs.body.as_ref(),
            })
            .collect()
    }

    fn dispatch(&self, request: &Request) -> Dispatch<'_> {
        match self.routes.lookup(request.method(), request.uri().path()) {
            Lookup::Found {
                value: route,
                parameters,
            } => match extract_arguments(&route.inputs, &parameters, request) {
                Ok(arguments) => Dispatch::Endpoint(route.endpoint.as_ref(), arguments),
                Err(errors) => Dispatch::Respond(validation_failed(errors)),
            },
            Lookup::MethodNotAllowed { allowed } => Dispatch::Respond(method_not_allowed(&allowed)),
            Lookup::NotFound => Dispatch::Respond(
                HttpError::new(StatusCode::NOT_FOUND, "Not Found").into_response(),
            ),
        }
    }

    pub fn handle(&self, request: Request) -> impl Future<Output = Response> + Send + '_ {
        let dispatch = self.dispatch(&request);
        async move {
            match dispatch {
                Dispatch::Endpoint(endpoint, arguments) => {
                    endpoint.call_boxed(request, arguments).await
                }
                Dispatch::Respond(response) => response,
            }
        }
    }
}

impl RequestHandler for Router {
    fn call(self: &Arc<Self>, request: Request) -> impl Future<Output = Response> + Send + 'static {
        let router = Arc::clone(self);
        async move { router.handle(request).await }
    }
}

fn method_not_allowed(allowed: &[Method]) -> Response {
    let mut error = HttpError::new(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed");
    let allowed = allowed
        .iter()
        .map(Method::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    // method names are HTTP tokens, so this always succeeds
    if let Ok(value) = HeaderValue::from_str(&allowed) {
        error = error.with_header(ALLOW, value);
    }
    error.into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use bytes::Bytes;

    use super::*;

    #[test]
    fn debug_output_lists_the_routes() {
        let mut router = Router::new();
        router
            .route(
                RouteDescription::new(Method::POST, "/items"),
                |_request: Request, _arguments: Arguments| async { Response::new(Bytes::new()) },
            )
            .unwrap();

        assert_eq!(format!("{router:?}"), "Router { routes: [POST /items] }");
    }
}
