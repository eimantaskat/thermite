use std::convert::Infallible;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::{Body, Frame, SizeHint};
use tokio::sync::mpsc::UnboundedSender;

use crate::background::Task;

// hyper drops the body once it has written the response, even one it sends without a body
pub(super) struct ResponseBody {
    body: Full<Bytes>,
    tasks: Vec<Task>,
    sent: UnboundedSender<Vec<Task>>,
}

impl ResponseBody {
    pub(super) fn new(body: Bytes, tasks: Vec<Task>, sent: UnboundedSender<Vec<Task>>) -> Self {
        Self {
            body: Full::new(body),
            tasks,
            sent,
        }
    }
}

impl Body for ResponseBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        Pin::new(&mut self.get_mut().body).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

impl Drop for ResponseBody {
    fn drop(&mut self) {
        if !self.tasks.is_empty() {
            // the receiver is gone only when the connection was closed at the shutdown timeout
            let _ = self.sent.send(mem::take(&mut self.tasks));
        }
    }
}
