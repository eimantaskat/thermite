use std::fmt;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

pub(crate) type Job = Box<dyn for<'py> FnOnce(Python<'py>) + Send>;

// hands work to the event loop's thread without the GIL, so a tokio worker never waits for it
#[derive(Default)]
pub(crate) struct EventLoopQueue {
    event_loop: Mutex<Option<Py<PyAny>>>,
    waiting: Mutex<Vec<Job>>,
    // whether a drain is due on the event loop, which then finds what is pushed without a wake-up
    scheduled: AtomicBool,
}

impl fmt::Debug for EventLoopQueue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventLoopQueue")
            .finish_non_exhaustive()
    }
}

impl EventLoopQueue {
    pub(crate) fn set_event_loop(&self, event_loop: Py<PyAny>) {
        *self
            .event_loop
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(event_loop);
    }

    pub(crate) fn push(self: &Arc<Self>, job: Job) {
        self.lock_waiting().push(job);
        if !self.scheduled.swap(true, Ordering::AcqRel) {
            let queue = Arc::clone(self);
            // telling the event loop takes the GIL, which can wait for as long as another thread keeps it
            tokio::task::spawn_blocking(move || {
                Python::attach(|py| queue.schedule(py, "call_soon_threadsafe"));
            });
        }
    }

    fn drain(self: &Arc<Self>, py: Python<'_>) {
        let jobs = mem::take(&mut *self.lock_waiting());
        if jobs.is_empty() {
            self.scheduled.store(false, Ordering::Release);
            // a push that saw the old flag did not schedule anything
            let missed = !self.lock_waiting().is_empty();
            if missed && !self.scheduled.swap(true, Ordering::AcqRel) {
                self.schedule(py, "call_soon");
            }
            return;
        }
        for job in jobs {
            job(py);
        }
        self.schedule(py, "call_soon");
    }

    fn schedule(self: &Arc<Self>, py: Python<'_>, method: &str) {
        if self.call_on_event_loop(py, method).is_err() {
            // the event loop is closed or was never set, and a request dropped here answers 500
            self.scheduled.store(false, Ordering::Release);
            drop(mem::take(&mut *self.lock_waiting()));
        }
    }

    fn call_on_event_loop(self: &Arc<Self>, py: Python<'_>, method: &str) -> PyResult<()> {
        let event_loop = self
            .event_loop
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|event_loop| event_loop.clone_ref(py))
            .ok_or_else(|| PyRuntimeError::new_err("no event loop"))?;
        let drain = Drain {
            queue: Arc::clone(self),
        };
        event_loop.call_method1(py, method, (drain,))?;
        Ok(())
    }

    fn lock_waiting(&self) -> MutexGuard<'_, Vec<Job>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// what the event loop is asked to call
#[pyclass(frozen, module = "thermite._thermite")]
struct Drain {
    queue: Arc<EventLoopQueue>,
}

#[pymethods]
impl Drain {
    fn __call__(&self, py: Python<'_>) {
        self.queue.drain(py);
    }
}
