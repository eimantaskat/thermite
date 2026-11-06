use http::{Method, StatusCode, Uri};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use thermite_core::{
    Arguments, Docs, OpenApi, Request, Response, ResponseDescription, RouteDescription, Router,
};

use crate::RouteError;
use crate::description::{Parameter, Schema};

#[pyclass(module = "thermite._thermite")]
#[derive(Debug, Default)]
pub(crate) struct CoreApp {
    router: Router,
}

#[pymethods]
impl CoreApp {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn add_route(
        &mut self,
        method: &str,
        path: &str,
        name: &str,
        parameters: Vec<Parameter>,
        body: Option<Schema>,
        response: Option<Schema>,
    ) -> PyResult<()> {
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let mut description = RouteDescription::new(method, path).with_name(name);
        for parameter in parameters {
            description = description.parameter(parameter.0);
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
        self.router
            .route(description, not_implemented)
            .map_err(|error| RouteError::new_err(error.to_string()))
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

// what a route answers until step 24 calls the Python handler
async fn not_implemented(_request: Request, _arguments: Arguments) -> Response {
    let mut response = Response::default();
    *response.status_mut() = StatusCode::NOT_IMPLEMENTED;
    response
}
