mod error;
mod path;

use std::fmt;
use std::future::Future;

use bytes::Bytes;
use http::header::{ALLOW, CONTENT_TYPE};
use http::{HeaderValue, Method, StatusCode};

use crate::handler::{Endpoint, Request, RequestHandler, Response};
use path::{PathPattern, decoded_segments};

pub use error::RouteError;
pub use path::PathParameters;

const NOT_FOUND_BODY: &str = r#"{"detail":"Not Found"}"#;
const METHOD_NOT_ALLOWED_BODY: &str = r#"{"detail":"Method Not Allowed"}"#;

#[derive(Debug, Default)]
pub struct Router {
    routes: Vec<Route>,
}

struct Route {
    method: Method,
    pattern: PathPattern,
    endpoint: Box<dyn Endpoint>,
}

impl fmt::Debug for Route {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.method, self.pattern.as_str())
    }
}

enum Lookup<'router> {
    Found {
        endpoint: &'router dyn Endpoint,
        parameters: PathParameters,
    },
    MethodNotAllowed {
        allowed: Vec<Method>,
    },
    NotFound,
}

impl Router {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn route(
        mut self,
        method: Method,
        path: &str,
        handler: impl RequestHandler,
    ) -> Result<Self, RouteError> {
        let pattern = PathPattern::parse(path)?;
        if let Some(existing) = self
            .routes
            .iter()
            .find(|route| route.method == method && route.pattern.matches_same_paths_as(&pattern))
        {
            return Err(RouteError::Conflict {
                method,
                path: path.to_owned(),
                existing: existing.pattern.as_str().to_owned(),
            });
        }

        let index = self
            .routes
            .partition_point(|route| route.pattern.cmp_specificity(&pattern).is_le());
        self.routes.insert(
            index,
            Route {
                method,
                pattern,
                endpoint: Box::new(handler),
            },
        );
        Ok(self)
    }

    fn lookup(&self, method: &Method, path: &str) -> Lookup<'_> {
        let segments = decoded_segments(path);
        let matching: Vec<&Route> = self
            .routes
            .iter()
            .filter(|route| route.pattern.matches(&segments))
            .collect();

        // like Starlette, a GET route also answers HEAD, and hyper leaves the body out
        let accepts = |route: &Route| {
            route.method == method || (*method == Method::HEAD && route.method == Method::GET)
        };
        let found = matching
            .iter()
            .copied()
            .find(|route| accepts(route))
            .map(|most_specific| {
                // an explicit HEAD route on the same pattern beats the GET fallback
                matching
                    .iter()
                    .copied()
                    .find(|route| {
                        route.method == method
                            && route.pattern.matches_same_paths_as(&most_specific.pattern)
                    })
                    .unwrap_or(most_specific)
            });

        match found {
            Some(route) => Lookup::Found {
                endpoint: route.endpoint.as_ref(),
                parameters: route.pattern.parameters(&segments),
            },
            None if matching.is_empty() => Lookup::NotFound,
            None => Lookup::MethodNotAllowed {
                allowed: allowed_methods(&matching),
            },
        }
    }
}

impl RequestHandler for Router {
    fn call(&self, mut request: Request) -> impl Future<Output = Response> + Send {
        let lookup = self.lookup(request.method(), request.uri().path());
        async move {
            match lookup {
                Lookup::Found {
                    endpoint,
                    parameters,
                } => {
                    request.extensions_mut().insert(parameters);
                    endpoint.call_boxed(request).await
                }
                Lookup::MethodNotAllowed { allowed } => method_not_allowed(&allowed),
                Lookup::NotFound => json_response(StatusCode::NOT_FOUND, NOT_FOUND_BODY),
            }
        }
    }
}

fn allowed_methods(routes: &[&Route]) -> Vec<Method> {
    let mut allowed = Vec::new();
    for route in routes {
        let implied = (route.method == Method::GET).then_some(Method::HEAD);
        for method in std::iter::once(route.method.clone()).chain(implied) {
            if !allowed.contains(&method) {
                allowed.push(method);
            }
        }
    }
    allowed
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

fn json_response(status: StatusCode, body: &'static str) -> Response {
    let mut response = Response::new(Bytes::from_static(body.as_bytes()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn answering(body: &'static str) -> impl RequestHandler {
        move |_request: Request| async move { Response::new(Bytes::from_static(body.as_bytes())) }
    }

    async fn body_for(router: &Router, method: Method, path: &str) -> Bytes {
        let mut request = Request::new(Bytes::new());
        *request.method_mut() = method;
        *request.uri_mut() = path.parse().unwrap();
        router.call(request).await.into_body()
    }

    #[tokio::test]
    async fn prefers_a_literal_segment_at_the_first_difference() {
        let router = Router::new()
            .route(Method::GET, "/{first}/b", answering("parameter first"))
            .unwrap()
            .route(Method::GET, "/a/{second}", answering("literal first"))
            .unwrap();

        assert_eq!(
            body_for(&router, Method::GET, "/a/b").await,
            "literal first"
        );
        assert_eq!(
            body_for(&router, Method::GET, "/x/b").await,
            "parameter first"
        );
    }

    #[tokio::test]
    async fn an_explicit_head_route_beats_the_get_fallback() {
        let router = Router::new()
            .route(Method::GET, "/items", answering("get"))
            .unwrap()
            .route(Method::HEAD, "/items", answering("head"))
            .unwrap();

        assert_eq!(body_for(&router, Method::HEAD, "/items").await, "head");
    }

    #[tokio::test]
    async fn head_goes_to_the_most_specific_route_even_through_the_get_fallback() {
        let router = Router::new()
            .route(Method::HEAD, "/items/{id}", answering("head parameter"))
            .unwrap()
            .route(Method::GET, "/items/all", answering("get literal"))
            .unwrap();

        assert_eq!(
            body_for(&router, Method::HEAD, "/items/all").await,
            "get literal"
        );
        assert_eq!(
            body_for(&router, Method::HEAD, "/items/5").await,
            "head parameter"
        );
    }

    #[test]
    fn lists_get_once_and_head_once_in_allowed_methods() {
        let router = Router::new()
            .route(Method::GET, "/items/{id}", answering(""))
            .unwrap()
            .route(Method::HEAD, "/items/{id}", answering(""))
            .unwrap()
            .route(Method::GET, "/items/all", answering(""))
            .unwrap();

        assert!(matches!(
            router.lookup(&Method::POST, "/items/all"),
            Lookup::MethodNotAllowed { allowed } if allowed == [Method::GET, Method::HEAD]
        ));
    }

    #[test]
    fn debug_output_lists_the_routes() {
        let router = Router::new()
            .route(Method::POST, "/items", answering(""))
            .unwrap();

        assert_eq!(format!("{router:?}"), "Router { routes: [POST /items] }");
    }
}
