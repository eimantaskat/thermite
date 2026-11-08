use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use bytes::Bytes;
use http::header::CONTENT_TYPE;
use http::{HeaderValue, StatusCode};
use pyo3::exceptions::PyBaseException;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use thermite_core::{Arguments, Request, Response, RouteHandler, attach_error};
use tokio::sync::oneshot;

use crate::body::{Build, Deadline, Plan, Progress};
use crate::description::Parameter;
use crate::event_loop::{EventLoopQueue, Job};
use crate::value::to_python;

// a body this size takes about 0.3 ms to build, past which a `def` route builds it on the event loop in turns
const LARGE_BODY: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunsOn {
    // a `def` handler may block, so it gets a thread of its own
    BlockingPool,
    EventLoop,
}

// the Python object that runs the handler, with what the core's arguments are called in its signature
#[derive(Debug)]
pub(crate) struct PythonRoute {
    route: Arc<Py<PyAny>>,
    parameters: Arc<[Parameter]>,
    body: Option<Arc<Plan>>,
    runs_on: RunsOn,
    queue: Arc<EventLoopQueue>,
}

impl PythonRoute {
    pub(crate) fn new(
        route: Py<PyAny>,
        parameters: Vec<Parameter>,
        body: Option<Arc<Plan>>,
        runs_on: RunsOn,
        queue: Arc<EventLoopQueue>,
    ) -> Self {
        Self {
            route: Arc::new(route),
            parameters: parameters.into(),
            body,
            runs_on,
            queue,
        }
    }
}

impl RouteHandler for PythonRoute {
    fn call(
        &self,
        request: Request,
        arguments: Arguments,
    ) -> impl Future<Output = Response> + Send {
        let (sender, receiver) = oneshot::channel();
        let respond = Respond::new(sender);
        let route = Arc::clone(&self.route);
        let queue = Arc::clone(&self.queue);
        let stage = Stage::Waiting {
            parameters: Arc::clone(&self.parameters),
            body: self.body.clone(),
            arguments,
        };
        let large_body = self.body.is_some() && request.body().len() >= LARGE_BODY;
        let runs_on = self.runs_on;
        async move {
            match (runs_on, large_body) {
                (RunsOn::EventLoop, _) => {
                    let call = Call::new(route, respond, stage, Then::Start);
                    queue.push(call.on_event_loop(Arc::clone(&queue)));
                }
                // taking the GIL can wait for as long as another thread keeps it, which would stall an async worker
                (RunsOn::BlockingPool, false) => {
                    let call = Call::new(route, respond, stage, Then::Start);
                    tokio::task::spawn_blocking(move || Python::attach(|py| call.run(py)));
                }
                // a pool thread gives up the GIL only every switch interval, so building there would hold up the loop
                (RunsOn::BlockingPool, true) => {
                    let (hand_over, handed) = oneshot::channel();
                    let starts = (Arc::clone(&route), respond.clone());
                    let call = Call::new(route, respond, stage, Then::HandOver(hand_over));
                    queue.push(call.on_event_loop(Arc::clone(&queue)));
                    if let Ok(arguments) = handed.await {
                        tokio::task::spawn_blocking(move || {
                            let (route, respond) = starts;
                            Python::attach(|py| start(py, &route, arguments, &respond));
                        });
                    }
                }
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

// one request on its way into Python
struct Call {
    route: Arc<Py<PyAny>>,
    respond: Respond,
    stage: Stage,
    then: Then,
}

enum Stage {
    Waiting {
        parameters: Arc<[Parameter]>,
        body: Option<Arc<Plan>>,
        arguments: Arguments,
    },
    Building {
        arguments: Py<PyDict>,
        argument: Py<PyString>,
        build: Build,
    },
}

// what happens once the handler's arguments are ready
enum Then {
    Start,
    HandOver(oneshot::Sender<Py<PyDict>>),
}

enum Advanced {
    Paused(Stage),
    Ready(Py<PyDict>),
}

impl Call {
    fn new(route: Arc<Py<PyAny>>, respond: Respond, stage: Stage, then: Then) -> Self {
        Self {
            route,
            respond,
            stage,
            then,
        }
    }

    fn run(self, py: Python<'_>) {
        let mut call = Some(self);
        // without a deadline a build never pauses, so this goes round once
        while let Some(next) = call {
            call = next.resume(py, None);
        }
    }

    fn on_event_loop(self, queue: Arc<EventLoopQueue>) -> Job {
        Box::new(move |py, turn_ends| {
            if let Some(paused) = self.resume(py, Some(turn_ends)) {
                let job = paused.on_event_loop(Arc::clone(&queue));
                queue.requeue(job);
            }
        })
    }

    // gets on with the request until the deadline, and gives it back if there is more to do
    fn resume(self, py: Python<'_>, pause_at: Option<Instant>) -> Option<Self> {
        let Self {
            route,
            respond,
            stage,
            then,
        } = self;
        match advance(py, stage, pause_at) {
            Ok(Advanced::Paused(stage)) => Some(Self::new(route, respond, stage, then)),
            Ok(Advanced::Ready(arguments)) => {
                match then {
                    Then::Start => start(py, &route, arguments, &respond),
                    // no one waits when the client is gone, and then no one needs the arguments
                    Then::HandOver(sender) => drop(sender.send(arguments)),
                }
                None
            }
            Err(error) => {
                fail(py, &route, &respond, error);
                None
            }
        }
    }
}

fn advance(py: Python<'_>, stage: Stage, pause_at: Option<Instant>) -> PyResult<Advanced> {
    let (arguments, body) = match stage {
        Stage::Waiting {
            parameters,
            body,
            mut arguments,
        } => {
            let values = parameters_to_python(py, &parameters, &arguments)?;
            let body = body.zip(arguments.take_body()).map(|(plan, body)| {
                let argument = plan.argument.clone_ref(py);
                (argument, Build::new(py, plan, body))
            });
            (values, body)
        }
        Stage::Building {
            arguments,
            argument,
            build,
        } => (arguments, Some((argument, build))),
    };
    if let Some((argument, build)) = body {
        match build.resume(py, &mut Deadline::new(pause_at))? {
            Progress::Paused(build) => {
                return Ok(Advanced::Paused(Stage::Building {
                    arguments,
                    argument,
                    build,
                }));
            }
            Progress::Built(value) => arguments.bind(py).set_item(argument, value)?,
        }
    }
    Ok(Advanced::Ready(arguments))
}

fn parameters_to_python(
    py: Python<'_>,
    parameters: &[Parameter],
    arguments: &Arguments,
) -> PyResult<Py<PyDict>> {
    let values = PyDict::new(py);
    for parameter in parameters {
        // one left out takes the default in the handler's signature
        if let Some(value) = arguments.parameter(parameter.location, &parameter.name) {
            values.set_item(&parameter.argument, to_python(py, value)?)?;
        }
    }
    Ok(values.unbind())
}

fn start(py: Python<'_>, route: &Py<PyAny>, arguments: Py<PyDict>, respond: &Respond) {
    if let Err(error) = route.call_method1(py, "start", (arguments, respond.clone())) {
        respond.finish(Err(error));
    }
}

// a body the handler's models reject is logged as the handler's own failures are
fn fail(py: Python<'_>, route: &Py<PyAny>, respond: &Respond, error: PyErr) {
    if let Err(error) = route.call_method1(py, "fail", (respond.clone(), error.into_value(py))) {
        respond.finish(Err(error));
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
