use std::future::Future;
use std::pin::pin;
use std::sync::{Arc, PoisonError};

use http::{HeaderValue, StatusCode};
use tokio::sync::{Mutex, oneshot};
use tokio_tungstenite::tungstenite::protocol::Role;

use super::CloseCode;
use super::socket::{Io, Shutdown, Socket};
use crate::background::Task;
use crate::handler::Response;

pub(crate) enum Decision {
    Accept { subprotocol: Option<HeaderValue> },
    Deny(Response),
}

pub(crate) struct HandOver {
    pub(super) socket: Arc<Mutex<Socket>>,
    pub(super) held: oneshot::Sender<()>,
}

// the rest of a handler that accepted, carried by its 101 to whoever upgrades the connection
pub(crate) struct Session {
    pub(super) handler: Task,
    pub(super) hand_over: oneshot::Sender<HandOver>,
}

// response extensions are Clone, which a session cannot be
#[derive(Clone)]
struct PendingSession(Arc<std::sync::Mutex<Option<Session>>>);

pub(crate) fn attach_session(response: &mut Response, session: Session) {
    response
        .extensions_mut()
        .insert(PendingSession(Arc::new(std::sync::Mutex::new(Some(
            session,
        )))));
}

pub(crate) fn take_session(response: &mut Response) -> Option<Session> {
    let pending = response.extensions_mut().remove::<PendingSession>()?;
    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
        return None;
    }
    pending
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
}

impl Session {
    // hands the upgraded connection to the handler, which runs as run_handler says, answering the code to close with
    pub(crate) async fn run<Running: Future<Output = CloseCode>>(
        self,
        io: Box<dyn Io>,
        shutdown: Shutdown,
        run_handler: impl FnOnce(Task) -> Running,
    ) {
        let socket = Arc::new(Mutex::new(
            Socket::new(io, Role::Server, shutdown.clone()).await,
        ));
        let (held, released) = oneshot::channel();
        let _ = self.hand_over.send(HandOver {
            socket: Arc::clone(&socket),
            held,
        });
        let finished = async {
            let code = run_handler(self.handler).await;
            // the handler may have handed its connection to a task that is still running
            let _ = released.await;
            code
        };
        let mut finished = pin!(finished);
        let code = tokio::select! {
            code = &mut finished => code,
            () = close_on_shutdown(&socket, shutdown) => finished.await,
        };
        socket.lock().await.close(code, "").await;
    }
}

// like uvicorn, which closes every WebSocket when it shuts down, whatever its handler is doing
async fn close_on_shutdown(socket: &Mutex<Socket>, mut shutdown: Shutdown) {
    shutdown.requested().await;
    socket
        .lock()
        .await
        .close(CloseCode::SERVICE_RESTART, "")
        .await;
}
