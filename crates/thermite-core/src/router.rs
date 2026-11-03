mod check;
mod extract;
mod path;
mod table;

use std::future::Future;
use std::sync::Arc;

use http::header::ALLOW;
use http::{HeaderValue, Method, StatusCode};

use crate::arguments::Arguments;
use crate::description::{Parameter, ResponseDescription, RouteDescription};
use crate::handler::{Endpoint, Request, RequestHandler, Response, RouteHandler};
use crate::http_error::HttpError;
use crate::route_error::RouteError;
use crate::schema::Schema;
use crate::security::SecuritySchemeDescription;
use crate::validation::validation_failed;
use crate::websocket::{
    AcceptKey, BoxedWebSocketHandler, WebSocketHandler, check_handshake, is_handshake, open,
    refusal,
};
use check::{Checked, Inputs, check_description};
use extract::extract_arguments;
use path::PathPattern;
use table::{Lookup, RouteTable};

#[derive(Debug)]
pub struct Router {
    routes: RouteTable<Route>,
    // like Starlette, a WebSocket route answers only handshakes, which no other route answers
    websockets: RouteTable<WebSocketRoute>,
}

struct Route {
    // the table orders routes by how specific they are, while the docs list them as registered
    registered: usize,
    name: Option<String>,
    tags: Vec<String>,
    inputs: Inputs,
    responses: Vec<ResponseDescription>,
    security: Vec<SecuritySchemeDescription>,
    endpoint: Box<dyn Endpoint>,
}

struct WebSocketRoute {
    inputs: Inputs,
    // the handler's future outlives the request once the connection is upgraded
    handler: Arc<dyn BoxedWebSocketHandler>,
}

// what the docs show of a route
pub(crate) struct Operation<'router> {
    pub(crate) method: &'router Method,
    pub(crate) path: &'router str,
    pub(crate) name: Option<&'router str>,
    pub(crate) tags: &'router [String],
    pub(crate) parameters: &'router [Parameter],
    pub(crate) body: Option<&'router Schema>,
    pub(crate) responses: &'router [ResponseDescription],
    pub(crate) security: &'router [SecuritySchemeDescription],
}

enum Dispatch<'router> {
    Endpoint(&'router dyn Endpoint, Arguments),
    WebSocket(
        &'router Arc<dyn BoxedWebSocketHandler>,
        Arguments,
        AcceptKey,
    ),
    // 404, 405 or 422, answered without calling a handler
    Respond(Response),
}

impl Default for Router {
    fn default() -> Self {
        Self {
            routes: RouteTable::new(),
            websockets: RouteTable::new(),
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
        let pattern = parse_path(&description)?;
        let method = description.method().clone();
        let name = description.name().map(str::to_owned);
        let tags = description.tags().to_vec();
        let Checked {
            inputs,
            responses,
            security,
        } = check_description(description, &pattern)?;
        let registered = self.routes.len();
        self.routes.insert(
            method,
            pattern,
            Route {
                registered,
                name,
                tags,
                inputs,
                responses,
                security,
                endpoint: Box::new(handler),
            },
        )
    }

    // what the description declares is read from the handshake, which has no body
    pub fn websocket(
        &mut self,
        description: RouteDescription,
        handler: impl WebSocketHandler,
    ) -> Result<(), RouteError> {
        let pattern = parse_path(&description)?;
        let path = description.path().to_owned();
        if description.method() != Method::GET {
            return Err(RouteError::WebSocketMethod {
                method: description.method().clone(),
                path,
            });
        }
        let Checked { inputs, .. } = check_description(description, &pattern)?;
        if inputs.body.is_some() {
            return Err(RouteError::WebSocketBody { path });
        }
        self.websockets.insert(
            Method::GET,
            pattern,
            WebSocketRoute {
                inputs,
                handler: Arc::new(handler),
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
                responses: &route.responses,
                security: &route.security,
            })
            .collect()
    }

    fn dispatch(&self, request: &Request) -> Dispatch<'_> {
        if is_handshake(request) {
            return self.dispatch_websocket(request);
        }
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

    // whatever keeps a WebSocket from being accepted answers 403, as uvicorn does
    fn dispatch_websocket(&self, request: &Request) -> Dispatch<'_> {
        let key = match check_handshake(request) {
            Ok(key) => key,
            Err(invalid) => return Dispatch::Respond(invalid.into_response()),
        };
        match self
            .websockets
            .lookup(request.method(), request.uri().path())
        {
            Lookup::Found {
                value: route,
                parameters,
            } => match extract_arguments(&route.inputs, &parameters, request) {
                Ok(arguments) => Dispatch::WebSocket(&route.handler, arguments, key),
                // FastAPI closes with 1008 before accepting, which uvicorn answers with 403
                Err(_) => Dispatch::Respond(refusal(StatusCode::FORBIDDEN)),
            },
            Lookup::MethodNotAllowed { .. } | Lookup::NotFound => {
                Dispatch::Respond(refusal(StatusCode::FORBIDDEN))
            }
        }
    }

    pub fn handle(&self, request: Request) -> impl Future<Output = Response> + Send + '_ {
        let dispatch = self.dispatch(&request);
        async move {
            match dispatch {
                Dispatch::Endpoint(endpoint, arguments) => {
                    endpoint.call_boxed(request, arguments).await
                }
                Dispatch::WebSocket(handler, arguments, key) => {
                    open(handler, request, arguments, key).await
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

fn parse_path(description: &RouteDescription) -> Result<PathPattern, RouteError> {
    if let Some(prefix) = description.invalid_prefix() {
        return Err(RouteError::InvalidPrefix {
            prefix: prefix.to_owned(),
            path: description.path().to_owned(),
        });
    }
    PathPattern::parse(description.path())
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
    use crate::security::ApiKeyLocation;

    #[test]
    fn debug_output_lists_the_routes() {
        let mut router = Router::new();
        router
            .route(
                RouteDescription::new(Method::POST, "/items"),
                |_request: Request, _arguments: Arguments| async { Response::new(Bytes::new()) },
            )
            .unwrap();

        assert_eq!(
            format!("{router:?}"),
            "Router { routes: [POST /items], websockets: [] }"
        );
    }

    #[test]
    fn refuses_an_api_key_read_from_an_invalid_header_name() {
        let mut router = Router::new();
        let description = RouteDescription::new(Method::GET, "/items").security(
            SecuritySchemeDescription::api_key(ApiKeyLocation::Header, "x key"),
        );

        let outcome = router.route(description, |_: Request, _: Arguments| async {
            Response::new(Bytes::new())
        });

        assert_eq!(
            outcome,
            Err(RouteError::InvalidApiKeyHeader {
                path: "/items".to_owned(),
                name: "x key".to_owned()
            })
        );
        assert_eq!(
            format!("{router:?}"),
            "Router { routes: [], websockets: [] }"
        );
    }
}
