use std::sync::Arc;

use http::{Method, StatusCode, Uri};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use thermite_core::{
    Docs, OpenApi, Request, Response, ResponseDescription, RouteDescription, Router,
};

use crate::RouteError;
use crate::description::{Parameter, Schema};
use crate::route::PythonRoute;
use crate::server::{PythonServer, ServedApp, TestServer};

#[pyclass(module = "thermite._thermite")]
#[derive(Debug, Default)]
pub(crate) struct CoreApp {
    // shared with what serves the app, and only written to while nothing does
    router: Arc<Router>,
}

#[pymethods]
impl CoreApp {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::too_many_arguments)] // mirrors what the Python side has read from a handler
    fn add_route(
        &mut self,
        method: &str,
        path: &str,
        name: &str,
        parameters: Vec<Parameter>,
        body: Option<Schema>,
        response: Option<Schema>,
        route: Py<PyAny>,
    ) -> PyResult<()> {
        let router = Arc::get_mut(&mut self.router).ok_or_else(|| {
            RouteError::new_err(format!(
                "cannot add `{path}` while the app is being served or a TestClient holds it"
            ))
        })?;
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let mut description = RouteDescription::new(method, path).with_name(name);
        for parameter in &parameters {
            description = description.parameter(parameter.description.clone());
        }
        if let Some(body) = body {
            description = description.body(body.0);
        }
        if let Some(response) = response {
            description = description.response(
                ResponseDescription::new(StatusCode::OK, "Successful Response")
                    .with_schema(response.0),
            );
        }
        router
            .route(description, PythonRoute::new(route, parameters))
            .map_err(|error| RouteError::new_err(error.to_string()))
    }

    fn server(&self, host: &str, port: u16) -> PyResult<PythonServer> {
        Ok(PythonServer::new(self.served()?, host, port))
    }

    fn test_server(&self) -> PyResult<TestServer> {
        TestServer::new(self.served()?)
    }

    fn openapi_json(&self) -> PyResult<String> {
        let docs = Docs::new(&OpenApi::new().without_docs(), &self.router)
            .map_err(|error| RouteError::new_err(error.to_string()))?;
        let mut request = Request::default();
        *request.uri_mut() = Uri::from_static("/openapi.json");
        let body = docs
            .respond(&request)
            .map(Response::into_body)
            .unwrap_or_default();
        String::from_utf8(body.to_vec()).map_err(|error| PyValueError::new_err(error.to_string()))
    }
}

impl CoreApp {
    fn served(&self) -> PyResult<ServedApp> {
        ServedApp::new(Arc::clone(&self.router))
    }
}
