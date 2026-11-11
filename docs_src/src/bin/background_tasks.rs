// ANCHOR: imports
use std::error::Error;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use thermite::{
    App, BackgroundTasks, Json, Model, Path, RouteError, Server, State, get, post, routes,
};
// ANCHOR_END: imports

// ANCHOR: outbox
#[derive(Default)]
struct Outbox(Mutex<Vec<String>>);

impl Outbox {
    fn send(&self, message: String) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message);
    }

    fn sent(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
// ANCHOR_END: outbox

// ANCHOR: handler
#[derive(Deserialize, Model)]
struct EmailPath {
    email: String,
}

#[derive(Serialize, Model)]
struct Message {
    message: String,
}

#[post("/send-notification/{email}")]
async fn send_notification(
    Path(EmailPath { email }): Path<EmailPath>,
    State(outbox): State<Outbox>,
    tasks: BackgroundTasks,
) -> Json<Message> {
    tasks.add(async move {
        outbox.send(format!("notification for {email}: some notification"));
    });
    Json(Message {
        message: "Notification sent in the background".to_owned(),
    })
}

#[get("/sent")]
async fn read_sent(State(outbox): State<Outbox>) -> Json<Vec<String>> {
    Json(outbox.sent())
}
// ANCHOR_END: handler

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new()
        .state(Outbox::default())
        .routes(routes![send_notification, read_sent])
}
// ANCHOR_END: app

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = Server::bind("127.0.0.1:8000").await?;
    server.serve(app()?).await?;
    Ok(())
}

// ANCHOR: test
#[cfg(test)]
mod tests {
    use thermite::TestClient;

    use super::*;

    #[tokio::test]
    async fn sends_the_notification_after_answering() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/send-notification/foo@example.com")
            .send()
            .await?;
        let sent = client.get("/sent").send().await?;

        assert_eq!(
            response.text()?,
            r#"{"message":"Notification sent in the background"}"#
        );
        assert_eq!(
            sent.text()?,
            r#"["notification for foo@example.com: some notification"]"#
        );
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test
