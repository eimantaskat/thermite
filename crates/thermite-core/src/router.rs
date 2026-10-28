mod check;
mod extract;
mod path;
mod table;

use std::future::Future;

use http::header::ALLOW;
use http::{HeaderValue, Method, StatusCode};

use crate::arguments::Arguments;
use crate::description::RouteDescription;
use crate::handler::{Endpoint, Request, RequestHandler, Response, RouteHandler};
use crate::response::json_response;
use crate::route_error::RouteError;
use crate::validation::validation_failed;
use check::check_description;
use extract::extract_arguments;
use path::PathPattern;
use table::{Lookup, RouteTable};

const NOT_FOUND_BODY: &str = r#"{"detail":"Not Found"}"#;
const METHOD_NOT_ALLOWED_BODY: &str = r#"{"detail":"Method Not Allowed"}"#;

#[derive(Debug)]
pub struct Router {
    routes: RouteTable<Route>,
}

struct Route {
    description: RouteDescription,
    endpoint: Box<dyn Endpoint>,
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

    pub fn route(
        mut self,
        description: RouteDescription,
        handler: impl RouteHandler,
    ) -> Result<Self, RouteError> {
        let pattern = PathPattern::parse(description.path())?;
        check_description(&description, &pattern)?;
        self.routes.insert(
            description.method().clone(),
            pattern,
            Route {
                description,
                endpoint: Box::new(handler),
            },
        )?;
        Ok(self)
    }

    fn dispatch(&self, request: &Request) -> Dispatch<'_> {
        match self.routes.lookup(request.method(), request.uri().path()) {
            Lookup::Found {
                value: route,
                parameters,
            } => match extract_arguments(&route.description, &parameters, request) {
                Ok(arguments) => Dispatch::Endpoint(route.endpoint.as_ref(), arguments),
                Err(errors) => Dispatch::Respond(validation_failed(errors)),
            },
            Lookup::MethodNotAllowed { allowed } => Dispatch::Respond(method_not_allowed(&allowed)),
            Lookup::NotFound => {
                Dispatch::Respond(json_response(StatusCode::NOT_FOUND, NOT_FOUND_BODY))
            }
        }
    }
}

impl RequestHandler for Router {
    fn call(&self, request: Request) -> impl Future<Output = Response> + Send {
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

fn method_not_allowed(allowed: &[Method]) -> Response {
    let mut response = json_response(StatusCode::METHOD_NOT_ALLOWED, METHOD_NOT_ALLOWED_BODY);
    let allowed = allowed
        .iter()
        .map(Method::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    // method names are HTTP tokens, so this always succeeds
    if let Ok(value) = HeaderValue::from_str(&allowed) {
        response.headers_mut().insert(ALLOW, value);
    }
    response
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use bytes::Bytes;

    use super::*;

    #[test]
    fn debug_output_lists_the_routes() {
        let router = Router::new()
            .route(
                RouteDescription::new(Method::POST, "/items"),
                |_request: Request, _arguments: Arguments| async { Response::new(Bytes::new()) },
            )
            .unwrap();

        assert_eq!(format!("{router:?}"), "Router { routes: [POST /items] }");
    }
}
