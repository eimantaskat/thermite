use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use bytes::Bytes;
use http::header::CONTENT_TYPE;
use http::{HeaderValue, StatusCode};
use pyo3::IntoPyObjectExt;
use pyo3::exceptions::PyBaseException;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use thermite_core::{Arguments, Request, Response, RouteHandler, Value, attach_error};
use tokio::sync::oneshot;

use crate::description::Parameter;
use crate::event_loop::EventLoopQueue;

#[derive(Debug, Clone)]
pub(crate) enum Dispatch {
    // a `def` handler may block, so it gets a thread of its own
    BlockingPool,
    EventLoop(Arc<EventLoopQueue>),
}

// the Python object that runs the handler, with what the core's arguments are called in its signature
#[derive(Debug)]
pub(crate) struct PythonRoute {
    route: Arc<Py<PyAny>>,
    parameters: Arc<[Parameter]>,
    dispatch: Dispatch,
}

impl PythonRoute {
    pub(crate) fn new(route: Py<PyAny>, parameters: Vec<Parameter>, dispatch: Dispatch) -> Self {
        Self {
            route: Arc::new(route),
            parameters: parameters.into(),
            dispatch,
        }
    }
}

impl RouteHandler for PythonRoute {
    fn call(
        &self,
        _request: Request,
        arguments: Arguments,
    ) -> impl Future<Output = Response> + Send {
        let route = Arc::clone(&self.route);
        let parameters = Arc::clone(&self.parameters);
        let dispatch = self.dispatch.clone();
        let (sender, receiver) = oneshot::channel();
        async move {
            let respond = Respond::new(sender);
            let job = move |py: Python<'_>| {
                if let Err(error) = start(py, &route, &parameters, &arguments, respond.clone()) {
                    respond.finish(Err(error));
                }
            };
            match dispatch {
                // taking the GIL can wait for as long as another thread keeps it, which would stall an async worker
                Dispatch::BlockingPool => {
                    tokio::task::spawn_blocking(move || Python::attach(job));
                }
                Dispatch::EventLoop(queue) => queue.push(Box::new(job)),
            }
            match receiver.await {
                Ok(Ok(body)) => json_response(body),
                Ok(Err(error)) => {
                    let mut response = internal_server_error();
                    attach_error(&mut response, Arc::new(error));
                    response
                }
                Err(_) => internal_server_error(),
            }
        }
    }
}

fn start(
    py: Python<'_>,
    route: &Py<PyAny>,
    parameters: &[Parameter],
    arguments: &Arguments,
    respond: Respond,
) -> PyResult<()> {
    let values = PyDict::new(py);
    for parameter in parameters {
        // one left out takes the default in the handler's signature
        if let Some(value) = arguments.parameter(parameter.location, &parameter.name) {
            values.set_item(&parameter.argument, to_python(py, value)?)?;
        }
    }
    let body = arguments
        .body()
        .map(|body| to_python(py, body))
        .transpose()?;
    route.call_method1(py, "start", (values, body, respond))?;
    Ok(())
}

fn to_python<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyAny>> {
    match value {
        Value::Null => Ok(py.None().into_bound(py)),
        Value::Bool(value) => value.into_bound_py_any(py),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                integer.into_bound_py_any(py)
            } else if let Some(integer) = number.as_u64() {
                integer.into_bound_py_any(py)
            } else {
                number.as_f64().into_bound_py_any(py)
            }
        }
        Value::String(value) => value.into_bound_py_any(py),
        Value::Array(items) => {
            let items = items
                .iter()
                .map(|item| to_python(py, item))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyList::new(py, items)?.into_any())
        }
        Value::Object(fields) => {
            let object = PyDict::new(py);
            for (name, value) in fields {
                object.set_item(name, to_python(py, value)?)?;
            }
            Ok(object.into_any())
        }
    }
}

fn json_response(body: Bytes) -> Response {
    let mut response = Response::new(body);
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn internal_server_error() -> Response {
    let mut response = Response::new(Bytes::from_static(b"Internal Server Error"));
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

type Outcome = Result<Bytes, PyErr>;

// handed to the route, which calls it from whichever thread the handler finishes on
#[pyclass(frozen, module = "thermite._thermite")]
#[derive(Debug, Clone)]
pub(crate) struct Respond {
    // once the last copy is dropped with nothing sent, the request answers 500 instead of waiting forever
    sender: Arc<Mutex<Option<oneshot::Sender<Outcome>>>>,
}

impl Respond {
    fn new(sender: oneshot::Sender<Outcome>) -> Self {
        Self {
            sender: Arc::new(Mutex::new(Some(sender))),
        }
    }

    fn finish(&self, outcome: Outcome) {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        // the receiver is gone when the client disconnected, and then no one needs the answer
        if let Some(sender) = sender {
            let _ = sender.send(outcome);
        }
    }
}

#[pymethods]
impl Respond {
    fn succeed(&self, body: &[u8]) {
        self.finish(Ok(Bytes::copy_from_slice(body)));
    }

    fn fail(&self, error: Bound<'_, PyBaseException>) {
        self.finish(Err(PyErr::from_value(error.into_any())));
    }
}
