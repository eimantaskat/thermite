use std::any::Any;
use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;

use crate::background::{Task, run_in_order};
use crate::handler::{Request, RequestHandler, Response};
use crate::response::internal_server_error;
use crate::websocket::CloseCode;

// like Starlette's ServerErrorMiddleware; the future is dropped, and state shared with other requests is left as by any panicking task
pub(super) async fn call_catching_panics(
    handler: &Arc<impl RequestHandler>,
    request: Request,
) -> Response {
    catching_panics(|| handler.call(request))
        .await
        .unwrap_or_else(|payload| {
            tracing::error!(panic = message(payload.as_ref()), "a handler panicked");
            internal_server_error()
        })
}

pub(super) async fn run_catching_panics(tasks: Vec<Task>) {
    if let Err(payload) = catching_panics(|| run_in_order(tasks)).await {
        tracing::error!(
            panic = message(payload.as_ref()),
            "a background task panicked, skipping the tasks after it"
        );
    }
}

// like uvicorn, a WebSocket whose handler panicked closes with 1011
pub(super) async fn run_session_catching_panics(handler: Task) -> CloseCode {
    match catching_panics(|| handler).await {
        Ok(()) => CloseCode::NORMAL,
        Err(payload) => {
            tracing::error!(
                panic = message(payload.as_ref()),
                "a WebSocket handler panicked"
            );
            CloseCode::INTERNAL_ERROR
        }
    }
}

async fn catching_panics<Started: Future>(
    start: impl FnOnce() -> Started,
) -> Result<Started::Output, Box<dyn Any + Send>> {
    let mut future = pin!(catch_unwind(AssertUnwindSafe(start))?);
    poll_fn(
        |cx| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(poll) => poll.map(Ok),
            Err(payload) => Poll::Ready(Err(payload)),
        },
    )
    .await
}

fn message(payload: &(dyn Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else {
        "a value that is not a string"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_message_of_a_panic_whatever_its_payload() {
        let literal: Box<dyn Any + Send> = Box::new("literal");
        let formatted: Box<dyn Any + Send> = Box::new(format!("formatted {}", 1));
        let other: Box<dyn Any + Send> = Box::new(42);

        assert_eq!(message(literal.as_ref()), "literal");
        assert_eq!(message(formatted.as_ref()), "formatted 1");
        assert_eq!(message(other.as_ref()), "a value that is not a string");
    }
}
