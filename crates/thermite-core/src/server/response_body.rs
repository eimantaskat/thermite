use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use tokio::sync::mpsc::UnboundedSender;

use super::panic::{poll_catching_panics, run_catching_panics};
use crate::background::Task;
use crate::body::{Body, BoxError};

// hyper drops the body once it has written the response, even one it sends without a body
pub(super) struct ResponseBody {
    body: Body,
    tasks: Vec<Task>,
    sent: UnboundedSender<Task>,
}

impl ResponseBody {
    pub(super) fn new(body: Body, tasks: Vec<Task>, sent: UnboundedSender<Task>) -> Self {
        Self { body, tasks, sent }
    }
}

impl HttpBody for ResponseBody {
    type Data = Bytes;
    type Error = BoxError;

    // an error or a panic in the stream makes hyper close the connection, the response being cut short
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
        let this = self.get_mut();
        let polled = poll_catching_panics(&mut this.body, cx);
        if let Poll::Ready(Some(Err(error))) = &polled {
            tracing::error!(%error, "a response stream failed, closing the connection");
            if !this.tasks.is_empty() {
                this.tasks.clear();
                warn_skipped_tasks();
            }
        }
        polled.map(|chunk| chunk.map(|chunk| chunk.map(Frame::data)))
    }

    fn is_end_stream(&self) -> bool {
        self.body.as_bytes().is_some_and(Bytes::is_empty)
    }

    fn size_hint(&self) -> SizeHint {
        match self.body.as_bytes() {
            Some(bytes) => SizeHint::with_exact(bytes.len() as u64),
            None => SizeHint::default(),
        }
    }
}

// as Starlette skips them when the body raises, though not when the client goes away
pub(super) fn warn_skipped_tasks() {
    tracing::warn!("skipping the background tasks of a response whose stream failed");
}

impl Drop for ResponseBody {
    fn drop(&mut self) {
        if !self.tasks.is_empty() {
            // the receiver is gone only when the connection was closed at the shutdown timeout
            let tasks = mem::take(&mut self.tasks);
            let _ = self.sent.send(Box::pin(run_catching_panics(tasks)));
        }
    }
}
