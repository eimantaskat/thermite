use std::future::Future;
use std::mem::ManuallyDrop;
use std::net::TcpListener;
use std::sync::Arc;

use bytes::Bytes;
use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
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
    shutdown: Arc<Notify>,
}

impl PythonServer {
    pub(crate) fn new(app: ServedApp) -> Self {
        Self {
            app,
            shutdown: Arc::new(Notify::new()),
        }
    }

    fn run(
        &self,
        py: Python<'_>,
        server: impl Future<Output = Result<Server, ServerError>> + Send,
    ) -> PyResult<()> {
        py.detach(|| {
            let runtime = Builder::new_multi_thread().enable_all().build()?;
            let outcome = runtime.block_on(async {
                let requested = || async { self.shutdown.notified().await };
                server
                    .await?
                    .serve_with_shutdown(self.app.clone(), requested)
                    .await
            });
            // the server already waited as long as it will, so a handler still running is abandoned
            runtime.shutdown_background();
            outcome.map_err(|error: ServerError| PyOSError::new_err(error.to_string()))
        })
    }
}

// takes the socket away from Python, which then no longer closes it
fn take_listener(socket: &Bound<'_, PyAny>) -> PyResult<TcpListener> {
    let socket_type = socket.py().import("socket")?.getattr("socket")?;
    if !socket.is_instance(&socket_type)? {
        return Err(PyTypeError::new_err("expected a socket.socket"));
    }
    // a closed socket detaches as -1
    let raw: u64 = socket
        .call_method0("detach")?
        .extract()
        .map_err(|_| PyValueError::new_err("the socket is closed"))?;
    listener_from_detached(raw)
}

#[cfg(unix)]
fn listener_from_detached(raw: u64) -> PyResult<TcpListener> {
    use std::os::fd::{FromRawFd, RawFd};

    let descriptor = RawFd::try_from(raw)?;
    // SAFETY: `detach` gave up the open descriptor it returned, so this is its only owner
    #[allow(unsafe_code)] // the only way to take a socket over from Python
    Ok(unsafe { TcpListener::from_raw_fd(descriptor) })
}

#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)] // the same signature as on Unix, where it can fail
fn listener_from_detached(raw: u64) -> PyResult<TcpListener> {
    use std::os::windows::io::FromRawSocket;

    // SAFETY: `detach` gave up the open socket it returned, so this is its only owner
    #[allow(unsafe_code)] // the only way to take a socket over from Python
    Ok(unsafe { TcpListener::from_raw_socket(raw) })
}

#[pymethods]
impl PythonServer {
    fn serve(&self, py: Python<'_>, host: &str, port: u16) -> PyResult<()> {
        let address = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        self.run(py, async move { Server::bind(address).await })
    }

    fn serve_socket(&self, py: Python<'_>, socket: &Bound<'_, PyAny>) -> PyResult<()> {
        let listener = take_listener(socket)?;
        self.run(py, Server::from_std(listener))
    }

    fn request_shutdown(&self) {
        self.shutdown.notify_one();
    }
}

#[pyclass(frozen, module = "thermite._thermite")]
#[derive(Debug)]
pub(crate) struct TestServer {
    runtime: ManuallyDrop<Runtime>,
    server: InProcessServer<ServedApp>,
}

impl TestServer {
    pub(crate) fn new(app: ServedApp) -> PyResult<Self> {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let server = runtime
            .block_on(InProcessServer::start(app))
            .map_err(|error| PyOSError::new_err(error.to_string()))?;
        Ok(Self {
            runtime: ManuallyDrop::new(runtime),
            server,
        })
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // SAFETY: `drop` runs once and nothing reads `runtime` after this
        #[allow(unsafe_code)] // the alternative is an `Option` that is never `None` while in use
        let runtime = unsafe { ManuallyDrop::take(&mut self.runtime) };
        // an ordinary drop waits for the blocking pool, where a thread can be waiting for the GIL held here
        runtime.shutdown_background();
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
