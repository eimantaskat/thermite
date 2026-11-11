mod check;
mod extract;
mod mount;
mod path;
mod table;

use std::future::Future;
use std::panic::resume_unwind;
use std::path::PathBuf;
use std::sync::Arc;

use http::header::ALLOW;
use http::{HeaderValue, Method, StatusCode};

use crate::arguments::Arguments;
use crate::description::{BodyEncoding, Parameter, ResponseDescription, RouteDescription};
use crate::handler::{Endpoint, Request, RequestHandler, Response, RouteHandler};
use crate::http_error::HttpError;
use crate::response::internal_server_error;
use crate::route_error::RouteError;
use crate::schema::Schema;
use crate::security::SecuritySchemeDescription;
use crate::static_files::{self, StaticFiles};
use crate::validation::validation_failed;
use crate::websocket::{
    AcceptKey, BoxedWebSocketHandler, WebSocketHandler, check_handshake, is_handshake, open,
    refusal,
};
use check::{Checked, Inputs, check_description};
use extract::{BodyRejection, ReadBody, content, extract_arguments, read_body};
use mount::Mount;
use path::PathPattern;
use table::{Lookup, RouteTable};

#[derive(Debug)]
pub struct Router {
    routes: RouteTable<Route>,
    // like Starlette, a WebSocket route answers only handshakes, which no other route answers
    websockets: RouteTable<WebSocketRoute>,
    // the deepest first, so a mount inside another answers for its own paths
    mounts: Vec<Mount>,
}

// a body this large takes long enough to read that the other requests on its worker would wait for it
const LARGE_BODY: usize = 64 * 1024;

struct Route {
    name: Option<String>,
    tags: Vec<String>,
    // shared with the thread that reads a large body
    inputs: Arc<Inputs>,
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
    pub(crate) body: Option<(&'router Schema, &'router BodyEncoding)>,
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
    Files(&'router Arc<StaticFiles>, Option<PathBuf>),
    // 404, 405 or 422, answered without calling a handler
    Respond(Response),
}

impl Default for Router {
    fn default() -> Self {
        Self {
            routes: RouteTable::new(),
            websockets: RouteTable::new(),
            mounts: Vec::new(),
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
        self.routes.insert(
            method,
            pattern,
            Route {
                name,
                tags,
                inputs: Arc::new(inputs),
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

    // answers the paths below it that no route matches, as a Starlette mount after its routes would
    pub fn mount(&mut self, path: impl Into<String>, files: StaticFiles) -> Result<(), RouteError> {
        let mount = Mount::new(path.into(), files)?;
        if self
            .mounts
            .iter()
            .any(|existing| existing.has_segments_of(&mount))
        {
            return Err(RouteError::MountConflict {
                path: mount.path().to_owned(),
            });
        }
        let index = self
            .mounts
            .partition_point(|existing| existing.depth() >= mount.depth());
        self.mounts.insert(index, mount);
        Ok(())
    }

    pub(crate) fn operations(&self) -> Vec<Operation<'_>> {
        self.routes
            .iter()
            .map(|(method, pattern, route)| Operation {
                method,
                path: pattern.as_str(),
                name: route.name.as_deref(),
                tags: &route.tags,
                parameters: &route.inputs.parameters,
                body: route
                    .inputs
                    .body
                    .as_ref()
                    .map(|schema| (schema, &route.inputs.encoding)),
                responses: &route.responses,
                security: &route.security,
            })
            .collect()
    }

    async fn dispatch(&self, request: &Request) -> Dispatch<'_> {
        if is_handshake(request) {
            return self.dispatch_websocket(request);
        }
        match self.routes.lookup(request.method(), request.uri().path()) {
            Lookup::Found {
                value: route,
                parameters,
            } => {
                let body = match read_request_body(&route.inputs, request).await {
                    Some(Ok(body)) => body,
                    Some(Err(BodyRejection::Invalid(errors))) => {
                        return Dispatch::Respond(validation_failed(errors));
                    }
                    Some(Err(BodyRejection::Malformed(error))) => {
                        return Dispatch::Respond(
                            HttpError::new(StatusCode::BAD_REQUEST, error.to_string())
                                .into_response(),
                        );
                    }
                    None => return Dispatch::Respond(internal_server_error()),
                };
                match extract_arguments(&route.inputs, &parameters, request, body) {
                    Ok(arguments) => Dispatch::Endpoint(route.endpoint.as_ref(), arguments),
                    Err(errors) => Dispatch::Respond(validation_failed(errors)),
                }
            }
            Lookup::MethodNotAllowed { allowed } => self
                .dispatch_files(request)
                .unwrap_or_else(|| Dispatch::Respond(method_not_allowed(&allowed))),
            Lookup::NotFound => self.dispatch_files(request).unwrap_or_else(|| {
                Dispatch::Respond(
                    HttpError::new(StatusCode::NOT_FOUND, "Not Found").into_response(),
                )
            }),
        }
    }

    fn dispatch_files(&self, request: &Request) -> Option<Dispatch<'_>> {
        let path = request.uri().path();
        self.mounts
            .iter()
            .find_map(|mount| mount.find(path))
            .map(|(files, relative)| Dispatch::Files(files, relative))
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
            } => {
                match extract_arguments(&route.inputs, &parameters, request, ReadBody::default()) {
                    Ok(arguments) => Dispatch::WebSocket(&route.handler, arguments, key),
                    // FastAPI closes with 1008 before accepting, which uvicorn answers with 403
                    Err(_) => Dispatch::Respond(refusal(StatusCode::FORBIDDEN)),
                }
            }
            Lookup::MethodNotAllowed { .. } | Lookup::NotFound => {
                Dispatch::Respond(refusal(StatusCode::FORBIDDEN))
            }
        }
    }

    // the signature promises Send, which an async fn would leave to inference
    pub fn handle(&self, request: Request) -> impl Future<Output = Response> + Send + '_ {
        self.respond(request)
    }

    async fn respond(&self, request: Request) -> Response {
        match self.dispatch(&request).await {
            Dispatch::Endpoint(endpoint, arguments) => {
                endpoint.call_boxed(request, arguments).await
            }
            Dispatch::WebSocket(handler, arguments, key) => {
                open(handler, request, arguments, key).await
            }
            Dispatch::Files(files, relative) => {
                static_files::respond(files, &request, relative).await
            }
            Dispatch::Respond(response) => response,
        }
    }
}

impl RequestHandler for Router {
    fn call(self: &Arc<Self>, request: Request) -> impl Future<Output = Response> + Send + 'static {
        let router = Arc::clone(self);
        async move { router.handle(request).await }
    }
}

// None when the runtime shut down before the body was read
async fn read_request_body(
    inputs: &Arc<Inputs>,
    request: &Request,
) -> Option<Result<ReadBody, BodyRejection>> {
    let content = content(request);
    if inputs.body.is_none() || request.body().len() < LARGE_BODY {
        return Some(read_body(inputs, request.body(), &content));
    }
    let inputs = Arc::clone(inputs);
    let body = request.body().clone();
    match tokio::task::spawn_blocking(move || read_body(&inputs, &body, &content)).await {
        Ok(read) => Some(read),
        // a decoder that panicked fails the request as it would have on the worker
        Err(error) => match error.try_into_panic() {
            Ok(payload) => resume_unwind(payload),
            Err(_cancelled) => None,
        },
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

    use super::*;
    use crate::security::ApiKeyLocation;

    #[test]
    fn debug_output_lists_the_routes() {
        let mut router = Router::new();
        router
            .route(
                RouteDescription::new(Method::POST, "/items"),
                |_request: Request, _arguments: Arguments| async { Response::default() },
            )
            .unwrap();

        assert_eq!(
            format!("{router:?}"),
            "Router { routes: [POST /items], websockets: [], mounts: [] }"
        );
    }

    #[test]
    fn refuses_an_api_key_read_from_an_invalid_header_name() {
        let mut router = Router::new();
        let description = RouteDescription::new(Method::GET, "/items").security(
            SecuritySchemeDescription::api_key(ApiKeyLocation::Header, "x key"),
        );

        let outcome = router.route(description, |_: Request, _: Arguments| async {
            Response::default()
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
            "Router { routes: [], websockets: [], mounts: [] }"
        );
    }
}
