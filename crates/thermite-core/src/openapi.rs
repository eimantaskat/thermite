mod document;
mod swagger;

use bytes::Bytes;
use http::header::CONTENT_TYPE;
use http::{HeaderValue, Method};

use crate::handler::{Request, Response};
use crate::router::Router;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OpenApiError {
    #[error("the docs path `{path}` must start with `/`")]
    MissingLeadingSlash { path: String },
    #[error("the OpenAPI document and the docs page cannot share the path `{path}`")]
    SamePath { path: String },
    #[error("the route `{method} {path}` can never run, because the docs answer there")]
    ShadowedRoute { method: Method, path: String },
    #[error(
        "the route `{method} {path}` uses a schema named `{name}`, but a different schema has that name"
    )]
    NameConflict {
        name: String,
        method: Method,
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenApi {
    title: String,
    version: String,
    openapi_path: Option<String>,
    docs_path: Option<String>,
}

impl Default for OpenApi {
    fn default() -> Self {
        Self {
            title: "Thermite".to_owned(),
            version: "0.1.0".to_owned(),
            openapi_path: Some("/openapi.json".to_owned()),
            docs_path: Some("/docs".to_owned()),
        }
    }
}

impl OpenApi {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    #[must_use]
    pub fn openapi_path(mut self, path: impl Into<String>) -> Self {
        self.openapi_path = Some(path.into());
        self
    }

    #[must_use]
    pub fn docs_path(mut self, path: impl Into<String>) -> Self {
        self.docs_path = Some(path.into());
        self
    }

    // like FastAPI, the docs page needs the document, so this turns both off
    #[must_use]
    pub fn without_openapi(mut self) -> Self {
        self.openapi_path = None;
        self
    }

    #[must_use]
    pub fn without_docs(mut self) -> Self {
        self.docs_path = None;
        self
    }
}

// the document and docs page of one router, built once since the routes no longer change
#[derive(Debug, Default)]
pub struct Docs {
    pages: Vec<Page>,
}

#[derive(Debug)]
struct Page {
    path: String,
    content_type: &'static str,
    body: Bytes,
}

impl Docs {
    pub fn new(openapi: &OpenApi, router: &Router) -> Result<Self, OpenApiError> {
        let Some(openapi_path) = &openapi.openapi_path else {
            return Ok(Self::default());
        };
        let openapi_path = checked(openapi_path)?;
        let docs_path = openapi.docs_path.as_deref().map(checked).transpose()?;
        if docs_path.as_ref() == Some(&openapi_path) {
            return Err(OpenApiError::SamePath { path: openapi_path });
        }
        let mut pages = Vec::new();
        if let Some(docs_path) = docs_path {
            pages.push(Page {
                body: swagger::page(&openapi.title, &openapi_path).into(),
                path: docs_path,
                content_type: "text/html; charset=utf-8",
            });
        }
        pages.push(Page {
            body: document::build(openapi, router)?.to_string().into(),
            path: openapi_path,
            content_type: "application/json",
        });
        let docs = Self { pages };
        docs.reject_shadowed_routes(router)?;
        Ok(docs)
    }

    // a route with parameters still runs for its other paths, so only a fixed path is shadowed
    fn reject_shadowed_routes(&self, router: &Router) -> Result<(), OpenApiError> {
        let shadowed = router.operations().into_iter().find(|operation| {
            (*operation.method == Method::GET || *operation.method == Method::HEAD)
                && !operation.path.contains('{')
                && self.pages.iter().any(|page| page.path == operation.path)
        });
        match shadowed {
            Some(operation) => Err(OpenApiError::ShadowedRoute {
                method: operation.method.clone(),
                path: operation.path.to_owned(),
            }),
            None => Ok(()),
        }
    }

    #[must_use]
    pub fn respond(&self, request: &Request) -> Option<Response> {
        if request.method() != Method::GET && request.method() != Method::HEAD {
            return None;
        }
        let page = self
            .pages
            .iter()
            .find(|page| page.path == request.uri().path())?;
        let mut response = Response::new(page.body.clone());
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(page.content_type));
        Some(response)
    }
}

fn checked(path: &str) -> Result<String, OpenApiError> {
    if path.starts_with('/') {
        Ok(path.to_owned())
    } else {
        Err(OpenApiError::MissingLeadingSlash {
            path: path.to_owned(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::arguments::Arguments;
    use crate::description::{Parameter, RouteDescription};
    use crate::schema::Schema;

    fn request(method: Method, path: &str) -> Request {
        http::Request::builder()
            .method(method)
            .uri(path)
            .body(Bytes::new())
            .unwrap()
    }

    fn docs(openapi: &OpenApi) -> Docs {
        Docs::new(openapi, &Router::new()).unwrap()
    }

    #[test]
    fn answers_get_and_head_on_its_paths_only() {
        let docs = docs(&OpenApi::new());

        let document = docs
            .respond(&request(Method::HEAD, "/openapi.json"))
            .unwrap();
        let page = docs.respond(&request(Method::GET, "/docs")).unwrap();

        assert_eq!(document.headers()[CONTENT_TYPE], "application/json");
        assert_eq!(page.headers()[CONTENT_TYPE], "text/html; charset=utf-8");
        assert!(docs.respond(&request(Method::POST, "/docs")).is_none());
        assert!(docs.respond(&request(Method::GET, "/docs/")).is_none());
    }

    #[test]
    fn without_the_document_there_is_no_docs_page() {
        let docs = docs(&OpenApi::new().without_openapi());

        assert!(
            docs.respond(&request(Method::GET, "/openapi.json"))
                .is_none()
        );
        assert!(docs.respond(&request(Method::GET, "/docs")).is_none());
    }

    #[test]
    fn rejects_paths_without_a_leading_slash() {
        for openapi in [
            OpenApi::new().openapi_path("openapi.json"),
            OpenApi::new().docs_path("docs"),
        ] {
            assert!(matches!(
                Docs::new(&openapi, &Router::new()),
                Err(OpenApiError::MissingLeadingSlash { .. })
            ));
        }
    }

    fn router(routes: impl IntoIterator<Item = RouteDescription>) -> Router {
        routes.into_iter().fold(Router::new(), |router, route| {
            router
                .route(route, |_: Request, _: Arguments| async {
                    Response::new(Bytes::new())
                })
                .unwrap()
        })
    }

    #[test]
    fn rejects_a_get_or_head_route_on_a_docs_path() {
        for (method, path) in [(Method::GET, "/docs"), (Method::HEAD, "/openapi.json")] {
            let outcome = Docs::new(
                &OpenApi::new(),
                &router([RouteDescription::new(method.clone(), path)]),
            );

            assert_eq!(
                outcome.err(),
                Some(OpenApiError::ShadowedRoute {
                    method,
                    path: path.to_owned()
                })
            );
        }
    }

    #[test]
    fn allows_routes_the_docs_leave_reachable() {
        let routes = router([
            RouteDescription::new(Method::POST, "/docs"),
            RouteDescription::new(Method::GET, "/{page}")
                .parameter(Parameter::path("page", Schema::string())),
            RouteDescription::new(Method::GET, "/openapi.json"),
        ]);

        assert!(Docs::new(&OpenApi::new().without_openapi(), &routes).is_ok());
        assert!(Docs::new(&OpenApi::new().openapi_path("/api.json"), &routes).is_ok());
    }

    #[test]
    fn names_the_route_whose_model_takes_a_built_in_name() {
        let routes = router([RouteDescription::new(Method::POST, "/errors")
            .body(Schema::model("ValidationError", []))]);

        assert_eq!(
            Docs::new(&OpenApi::new(), &routes).err(),
            Some(OpenApiError::NameConflict {
                name: "ValidationError".to_owned(),
                method: Method::POST,
                path: "/errors".to_owned()
            })
        );
    }

    #[test]
    fn rejects_the_document_and_the_page_on_one_path() {
        let openapi = OpenApi::new().docs_path("/openapi.json");

        assert_eq!(
            Docs::new(&openapi, &Router::new()).err(),
            Some(OpenApiError::SamePath {
                path: "/openapi.json".to_owned()
            })
        );
    }
}
