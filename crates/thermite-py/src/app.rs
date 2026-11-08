use std::sync::Arc;

use http::{Method, StatusCode, Uri};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use thermite_core::{
    Docs, OpenApi, Request, Response, ResponseDescription, RouteDescription, Router,
};

use crate::RouteError;
use crate::body::Body;
use crate::description::{Parameter, Schema};
use crate::event_loop::EventLoopQueue;
use crate::route::{PythonRoute, RunsOn};
use crate::server::{PythonServer, ServedApp, TestServer};

#[pyclass(module = "thermite._thermite")]
#[derive(Debug, Default)]
pub(crate) struct CoreApp {
    // shared with what serves the app, and only written to while nothing does
    router: Arc<Router>,
    event_loop_queue: Arc<EventLoopQueue>,
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
        body: Option<Bound<'_, Body>>,
        response: Option<Schema>,
        route: Py<PyAny>,
        on_event_loop: bool,
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
        let plan = body.as_ref().map(|body| Arc::clone(&body.get().plan));
        if let Some(body) = body {
            description = description.body(body.get().schema.0.clone());
        }
        if let Some(response) = response {
            description = description.response(
                ResponseDescription::new(StatusCode::OK, "Successful Response")
                    .with_schema(response.0),
            );
        }
        let runs_on = if on_event_loop {
            RunsOn::EventLoop
        } else {
            RunsOn::BlockingPool
        };
        let route = PythonRoute::new(
            route,
            parameters,
            plan,
            runs_on,
            Arc::clone(&self.event_loop_queue),
        );
        router
            .route(description, route)
            .map_err(|error| RouteError::new_err(error.to_string()))
    }

    fn server(&self, event_loop: Py<PyAny>) -> PyResult<PythonServer> {
        Ok(PythonServer::new(self.served(event_loop)?))
    }

    fn test_server(&self, event_loop: Py<PyAny>) -> PyResult<TestServer> {
        TestServer::new(self.served(event_loop)?)
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
    fn served(&self, event_loop: Py<PyAny>) -> PyResult<ServedApp> {
        self.event_loop_queue.set_event_loop(event_loop);
        ServedApp::new(Arc::clone(&self.router))
    }
}
