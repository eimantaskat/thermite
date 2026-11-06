use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use thermite_core::{
    Docs, InProcessServer, OpenApi, Request, RequestHandler, Response, Router, Server, ServerError,
    response_error,
};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::Notify;

use crate::RouteError;

#[derive(Debug, Clone)]
pub(crate) struct ServedApp {
    router: Arc<Router>,
    docs: Arc<Docs>,
}

impl ServedApp {
    pub(crate) fn new(router: Arc<Router>) -> PyResult<Self> {
        let docs = Docs::new(&OpenApi::new(), &router)
            .map_err(|error| RouteError::new_err(error.to_string()))?;
        Ok(Self {
            router,
            docs: Arc::new(docs),
        })
    }
}

impl RequestHandler for ServedApp {
    fn call(self: &Arc<Self>, request: Request) -> impl Future<Output = Response> + Send + 'static {
        let app = Arc::clone(self);
        async move {
            match app.docs.respond(&request) {
                Some(response) => response,
                None => app.router.handle(request).await,
            }
        }
    }
}

#[pyclass(frozen, name = "Server", module = "thermite._thermite")]
#[derive(Debug)]
pub(crate) struct PythonServer {
    app: ServedApp,
    address: String,
    shutdown: Arc<Notify>,
}

impl PythonServer {
    pub(crate) fn new(app: ServedApp, host: &str, port: u16) -> Self {
        let address = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        Self {
            app,
            address,
            shutdown: Arc::new(Notify::new()),
        }
    }
}

#[pymethods]
impl PythonServer {
    fn serve(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| {
            let runtime = Builder::new_multi_thread().enable_all().build()?;
            let served = runtime.block_on(async {
                let server = Server::bind(self.address.as_str()).await?;
                let requested = || async { self.shutdown.notified().await };
                server
                    .serve_with_shutdown(self.app.clone(), requested)
                    .await
            });
            // the server already waited as long as it will, so a handler still running is abandoned
            runtime.shutdown_background();
            served.map_err(|error: ServerError| PyOSError::new_err(error.to_string()))
        })
    }

    fn request_shutdown(&self) {
        self.shutdown.notify_one();
    }
}

#[pyclass(frozen, module = "thermite._thermite")]
#[derive(Debug)]
pub(crate) struct TestServer {
    runtime: Runtime,
    server: InProcessServer<ServedApp>,
}

impl TestServer {
    pub(crate) fn new(app: ServedApp) -> PyResult<Self> {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let server = runtime
            .block_on(InProcessServer::start(app))
            .map_err(|error| PyOSError::new_err(error.to_string()))?;
        Ok(Self { runtime, server })
    }
}

type TestResponse<'py> = (
    u16,
    Vec<(String, String)>,
    Bound<'py, PyBytes>,
    Option<Bound<'py, PyAny>>,
);

#[pymethods]
impl TestServer {
    fn send<'py>(
        &self,
        py: Python<'py>,
        method: &str,
        target: &str,
        headers: Vec<(String, String)>,
        body: &[u8],
    ) -> PyResult<TestResponse<'py>> {
        let mut request = http::Request::builder().method(method).uri(target);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let request = request
            .body(Bytes::copy_from_slice(body))
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let response = py.detach(|| self.runtime.block_on(self.server.send(request)));
        let error = response_error(&response)
            .and_then(|error| error.downcast_ref::<PyErr>())
            .map(|error| error.value(py).clone().into_any());
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        Ok((
            response.status().as_u16(),
            headers,
            PyBytes::new(py, response.body()),
            error,
        ))
    }
}
