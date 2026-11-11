use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
    ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, ORIGIN, VARY,
};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};

use super::vary::append_vary;
use super::{Middleware, Next};
use crate::handler::{Request, Response};
use crate::response::plain_text;

const ANY_METHOD: [&str; 8] = [
    "DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT", "QUERY",
];
// browsers send these without asking, and Starlette lists them in every preflight answer
const SAFELISTED_HEADERS: [&str; 4] = [
    "Accept",
    "Accept-Language",
    "Content-Language",
    "Content-Type",
];
const PREFLIGHT_VARY: HeaderValue = HeaderValue::from_static(
    "Origin, Access-Control-Request-Method, Access-Control-Request-Headers, Access-Control-Request-Private-Network",
);
const REQUEST_PRIVATE_NETWORK: HeaderName =
    HeaderName::from_static("access-control-request-private-network");
const ALLOW_PRIVATE_NETWORK: HeaderName =
    HeaderName::from_static("access-control-allow-private-network");
const TRUE: HeaderValue = HeaderValue::from_static("true");
const ANY: HeaderValue = HeaderValue::from_static("*");

type OriginPredicate = Arc<dyn Fn(&str) -> bool + Send + Sync>;

// Starlette's CORSMiddleware: answers preflight requests and tells browsers which origins may read a response
#[derive(Clone)]
pub struct CorsMiddleware {
    origins: Allowed<String>,
    origin_predicate: Option<OriginPredicate>,
    methods: Allowed<Method>,
    headers: Allowed<HeaderName>,
    credentials: bool,
    private_network: bool,
    expose_headers: Vec<HeaderName>,
    max_age: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Allowed<T> {
    Any,
    Listed(Vec<T>),
}

impl CorsMiddleware {
    // Starlette's defaults: no origin, only GET, preflights cached for 10 minutes
    #[must_use]
    pub fn new() -> Self {
        Self {
            origins: Allowed::Listed(Vec::new()),
            origin_predicate: None,
            methods: Allowed::Listed(vec![Method::GET]),
            headers: Allowed::Listed(Vec::new()),
            credentials: false,
            private_network: false,
            expose_headers: Vec::new(),
            max_age: Duration::from_secs(600),
        }
    }

    // "*" among them allows any origin, as in Starlette
    #[must_use]
    pub fn allow_origins<Origin: Into<String>>(
        mut self,
        origins: impl IntoIterator<Item = Origin>,
    ) -> Self {
        let origins: Vec<String> = origins.into_iter().map(Into::into).collect();
        self.origins = if origins.iter().any(|origin| origin == "*") {
            Allowed::Any
        } else {
            Allowed::Listed(origins)
        };
        self
    }

    // in place of Starlette's allow_origin_regex, also allowing the listed origins
    #[must_use]
    pub fn allow_origin_fn(
        mut self,
        predicate: impl Fn(&str) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.origin_predicate = Some(Arc::new(predicate));
        self
    }

    #[must_use]
    pub fn allow_methods(mut self, methods: impl IntoIterator<Item = Method>) -> Self {
        self.methods = Allowed::Listed(methods.into_iter().collect());
        self
    }

    // Starlette's allow_methods=["*"]
    #[must_use]
    pub fn allow_any_method(mut self) -> Self {
        self.methods = Allowed::Any;
        self
    }

    #[must_use]
    pub fn allow_headers(mut self, headers: impl IntoIterator<Item = HeaderName>) -> Self {
        self.headers = Allowed::Listed(headers.into_iter().collect());
        self
    }

    // Starlette's allow_headers=["*"], which allows whatever headers a preflight asks for
    #[must_use]
    pub fn allow_any_header(mut self) -> Self {
        self.headers = Allowed::Any;
        self
    }

    #[must_use]
    pub fn allow_credentials(mut self, allow: bool) -> Self {
        self.credentials = allow;
        self
    }

    #[must_use]
    pub fn allow_private_network(mut self, allow: bool) -> Self {
        self.private_network = allow;
        self
    }

    #[must_use]
    pub fn expose_headers(mut self, headers: impl IntoIterator<Item = HeaderName>) -> Self {
        self.expose_headers = headers.into_iter().collect();
        self
    }

    // sent in whole seconds
    #[must_use]
    pub fn max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    fn allows_any_origin(&self) -> bool {
        self.origins == Allowed::Any
    }

    fn allows_origin(&self, origin: &HeaderValue) -> bool {
        let Ok(origin) = origin.to_str() else {
            return self.allows_any_origin();
        };
        match &self.origins {
            Allowed::Any => true,
            Allowed::Listed(listed) => {
                self.origin_predicate
                    .as_ref()
                    .is_some_and(|predicate| predicate(origin))
                    || listed.iter().any(|allowed| allowed == origin)
            }
        }
    }

    fn allows_method(&self, requested: &HeaderValue) -> bool {
        let requested = requested.as_bytes();
        match &self.methods {
            Allowed::Any => ANY_METHOD
                .iter()
                .any(|method| method.as_bytes() == requested),
            Allowed::Listed(listed) => listed
                .iter()
                .any(|method| method.as_str().as_bytes() == requested),
        }
    }

    // each name, trimmed and in any case, must be allowed or safelisted, an empty one included
    fn allows_headers(listed: &[HeaderName], requested: &HeaderValue) -> bool {
        let Ok(requested) = requested.to_str() else {
            return false;
        };
        requested.split(',').map(str::trim).all(|name| {
            SAFELISTED_HEADERS
                .iter()
                .copied()
                .chain(listed.iter().map(HeaderName::as_str))
                .any(|allowed| allowed.eq_ignore_ascii_case(name))
        })
    }

    fn preflight(&self, origin: &HeaderValue, request: &HeaderMap) -> Response {
        let mut headers = HeaderMap::new();
        headers.insert(VARY, PREFLIGHT_VARY);
        // a credentialed request may not be answered with "*"
        let explicit_origin = !self.allows_any_origin() || self.credentials;
        if !explicit_origin {
            headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, ANY);
        }
        if let Ok(methods) = HeaderValue::try_from(self.methods_value()) {
            headers.insert(ACCESS_CONTROL_ALLOW_METHODS, methods);
        }
        headers.insert(
            ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from(self.max_age.as_secs()),
        );
        if let Allowed::Listed(listed) = &self.headers
            && let Ok(allowed) = HeaderValue::try_from(allowed_headers_value(listed))
        {
            headers.insert(ACCESS_CONTROL_ALLOW_HEADERS, allowed);
        }
        if self.credentials {
            headers.insert(ACCESS_CONTROL_ALLOW_CREDENTIALS, TRUE);
        }

        let mut failures = Vec::new();
        if !self.allows_origin(origin) {
            failures.push("origin");
        } else if explicit_origin {
            headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
        }
        if !request
            .get(ACCESS_CONTROL_REQUEST_METHOD)
            .is_some_and(|method| self.allows_method(method))
        {
            failures.push("method");
        }
        match (&self.headers, request.get(ACCESS_CONTROL_REQUEST_HEADERS)) {
            (_, None) => {}
            (Allowed::Any, Some(requested)) => {
                headers.insert(ACCESS_CONTROL_ALLOW_HEADERS, requested.clone());
            }
            (Allowed::Listed(listed), Some(requested)) => {
                if !Self::allows_headers(listed, requested) {
                    failures.push("headers");
                }
            }
        }
        if request.contains_key(REQUEST_PRIVATE_NETWORK) {
            if self.private_network {
                headers.insert(ALLOW_PRIVATE_NETWORK, TRUE);
            } else {
                failures.push("private-network");
            }
        }

        // the browser enforces the policy, so the status only tells a developer what failed
        let mut response = if failures.is_empty() {
            plain_text(StatusCode::OK, "OK")
        } else {
            let text = format!("Disallowed CORS {}", failures.join(", "));
            plain_text(StatusCode::BAD_REQUEST, text)
        };
        response.headers_mut().extend(headers);
        response
    }

    fn methods_value(&self) -> String {
        match &self.methods {
            Allowed::Any => ANY_METHOD.join(", "),
            Allowed::Listed(listed) => listed
                .iter()
                .map(Method::as_str)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    fn add_simple_headers(&self, headers: &mut HeaderMap, origin: Option<&HeaderValue>) {
        if let Some(origin) = origin {
            if self.allows_any_origin() {
                headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, ANY);
            }
            if self.credentials {
                headers.insert(ACCESS_CONTROL_ALLOW_CREDENTIALS, TRUE);
            }
            if !self.expose_headers.is_empty() {
                let exposed = self
                    .expose_headers
                    .iter()
                    .map(HeaderName::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Ok(exposed) = HeaderValue::try_from(exposed) {
                    headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, exposed);
                }
            }
            // Starlette mirrors any origin when credentials are allowed with "*"
            let mirrored = if self.allows_any_origin() {
                self.credentials
            } else {
                self.allows_origin(origin)
            };
            if mirrored {
                headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
            }
        }
        append_vary(headers, "Origin");
    }
}

// sorted as Starlette sorts them, the safelisted ones capitalized
fn allowed_headers_value(listed: &[HeaderName]) -> String {
    let mut names: Vec<&str> = SAFELISTED_HEADERS.to_vec();
    for name in listed {
        if !names
            .iter()
            .any(|known| known.eq_ignore_ascii_case(name.as_str()))
        {
            names.push(name.as_str());
        }
    }
    names.sort_unstable();
    names.join(", ")
}

impl Default for CorsMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl Middleware for CorsMiddleware {
    async fn call(&self, request: Request, next: Next) -> Response {
        let origin = request.headers().get(ORIGIN).cloned();
        if let Some(origin) = &origin
            && request.method() == Method::OPTIONS
            && request
                .headers()
                .contains_key(ACCESS_CONTROL_REQUEST_METHOD)
        {
            return self.preflight(origin, request.headers());
        }
        let mut response = next.run(request).await;
        self.add_simple_headers(response.headers_mut(), origin.as_ref());
        response
    }
}

impl fmt::Debug for CorsMiddleware {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CorsMiddleware")
            .field("origins", &self.origins)
            .field("origin_fn", &self.origin_predicate.is_some())
            .field("methods", &self.methods)
            .field("headers", &self.headers)
            .field("credentials", &self.credentials)
            .field("private_network", &self.private_network)
            .field("expose_headers", &self.expose_headers)
            .field("max_age", &self.max_age)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_safelisted_headers_with_the_allowed_ones_once_each() {
        let listed = [
            HeaderName::from_static("x-token"),
            HeaderName::from_static("accept"),
            HeaderName::from_static("x-token"),
        ];

        assert_eq!(
            allowed_headers_value(&listed),
            "Accept, Accept-Language, Content-Language, Content-Type, x-token"
        );
    }

    #[test]
    fn an_origin_that_is_not_text_is_allowed_only_by_any_origin() {
        let origin = HeaderValue::from_bytes(b"https://\xe9.example").unwrap_or(ANY);
        let listed = CorsMiddleware::new().allow_origin_fn(|_| true);

        assert!(!listed.allows_origin(&origin));
        assert!(
            CorsMiddleware::new()
                .allow_origins(["*"])
                .allows_origin(&origin)
        );
        assert!(!CorsMiddleware::allows_headers(&[], &origin));
    }

    #[test]
    fn debug_output_shows_the_settings() {
        let cors = CorsMiddleware::default().allow_origin_fn(|_| true);

        assert_eq!(
            format!("{cors:?}"),
            "CorsMiddleware { origins: Listed([]), origin_fn: true, methods: Listed([GET]), \
             headers: Listed([]), credentials: false, private_network: false, \
             expose_headers: [], max_age: 600s }"
        );
    }
}
