use std::pin::Pin;
use std::task::{Context, Poll, ready};

use bytes::Bytes;
use futures_util::Stream;

use super::Panics;
use crate::background::{Task, run_in_order};
use crate::body::{Body, BoxError};
use crate::server::panic::{poll_catching_panics, run_catching_panics};
use crate::server::response_body::warn_skipped_tasks;

// a response body that runs the background tasks once it has been read, as the server runs them once it has sent it
pub(super) struct Streamed {
    body: Option<Body>,
    tasks: Option<Task>,
    error: Option<BoxError>,
    panics: Panics,
}

impl Streamed {
    pub(super) fn new(body: Body, tasks: Vec<Task>, panics: Panics) -> Self {
        let tasks: Option<Task> = match (tasks.is_empty(), panics) {
            (true, _) => None,
            (false, Panics::Reach) => Some(Box::pin(run_in_order(tasks))),
            (false, Panics::Answer500) => Some(Box::pin(run_catching_panics(tasks))),
        };
        Self {
            body: Some(body),
            tasks,
            error: None,
            panics,
        }
    }
}

impl Stream for Streamed {
    type Item = Result<Bytes, BoxError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Some(body) = &mut this.body {
            let polled = match this.panics {
                Panics::Reach => Pin::new(body).poll_next(cx),
                Panics::Answer500 => poll_catching_panics(body, cx),
            };
            match ready!(polled) {
                Some(Ok(chunk)) => return Poll::Ready(Some(Ok(chunk))),
                Some(Err(error)) => {
                    this.error = Some(error);
                    if this.tasks.take().is_some() {
                        warn_skipped_tasks();
                    }
                }
                None => {}
            }
            this.body = None;
        }
        if let Some(tasks) = &mut this.tasks {
            ready!(tasks.as_mut().poll(cx));
            this.tasks = None;
        }
        Poll::Ready(this.error.take().map(Err))
    }
}

// a body the test stops reading leaves its tasks to the runtime, as a dropped connection does on the server
impl Drop for Streamed {
    fn drop(&mut self) {
        if let Some(tasks) = self.tasks.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(tasks);
        }
    }
}
