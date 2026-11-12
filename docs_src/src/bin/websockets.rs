// a dependency is written the way users write one, as an async fn that may not await
#![allow(clippy::unused_async_trait_impl)]

// ANCHOR: imports
use std::error::Error;
use std::sync::{Mutex, PoisonError};

use serde::Deserialize;
use thermite::{
    App, Cookie, Dependency, Depends, HttpError, Json, Model, Path, Query, RouteError, Server,
    State, StatusCode, WebSocket, WebSocketDisconnect, get, routes, websocket,
};
// ANCHOR_END: imports

// ANCHOR: echo
#[websocket("/ws")]
async fn websocket_endpoint(socket: WebSocket) -> Result<(), WebSocketDisconnect> {
    let mut connection = socket.accept().await?;
    loop {
        let data = connection.receive_text().await?;
        connection
            .send_text(format!("Message text was: {data}"))
            .await?;
    }
}
// ANCHOR_END: echo

// ANCHOR: dependency
#[derive(Deserialize, Model)]
struct Session {
    session: Option<String>,
}

#[derive(Deserialize, Model)]
struct Token {
    token: Option<String>,
}

#[derive(Clone)]
struct CookieOrToken(String);

impl Dependency for CookieOrToken {
    type Needs = (Cookie<Session>, Query<Token>);
    type Rejection = HttpError;

    async fn resolve((Cookie(cookie), Query(query)): Self::Needs) -> Result<Self, HttpError> {
        cookie
            .session
            .or(query.token)
            .map(Self)
            .ok_or_else(|| HttpError::new(StatusCode::FORBIDDEN, "Not authenticated"))
    }
}

#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: String,
}

#[derive(Deserialize, Model)]
struct Paging {
    q: Option<i64>,
}

#[websocket("/items/{item_id}/ws")]
async fn item_websocket(
    socket: WebSocket,
    Path(ItemPath { item_id }): Path<ItemPath>,
    Query(Paging { q }): Query<Paging>,
    Depends(CookieOrToken(credentials)): Depends<CookieOrToken>,
) -> Result<(), WebSocketDisconnect> {
    let mut connection = socket.accept().await?;
    loop {
        let data = connection.receive_text().await?;
        connection
            .send_text(format!(
                "Session cookie or query token value is: {credentials}"
            ))
            .await?;
        if let Some(q) = q {
            connection
                .send_text(format!("Query parameter q is: {q}"))
                .await?;
        }
        connection
            .send_text(format!("Message text was: {data}, for item ID: {item_id}"))
            .await?;
    }
}
// ANCHOR_END: dependency

// ANCHOR: disconnect
#[derive(Default)]
struct ChatLog(Mutex<Vec<String>>);

impl ChatLog {
    fn add(&self, line: String) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
    }

    fn lines(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[get("/chat/log")]
async fn chat_log(State(log): State<ChatLog>) -> Json<Vec<String>> {
    Json(log.lines())
}

#[derive(Deserialize, Model)]
struct Client {
    client_id: i64,
}

#[websocket("/chat/{client_id}")]
async fn chat(
    socket: WebSocket,
    Path(Client { client_id }): Path<Client>,
    State(log): State<ChatLog>,
) {
    let Ok(mut connection) = socket.accept().await else {
        return;
    };
    let disconnect = loop {
        let text = match connection.receive_text().await {
            Ok(text) => text,
            Err(disconnect) => break disconnect,
        };
        log.add(format!("Client #{client_id} says: {text}"));
        if let Err(disconnect) = connection.send_text(format!("You wrote: {text}")).await {
            break disconnect;
        }
    };
    log.add(format!(
        "Client #{client_id} left the chat with the code {}",
        disconnect.code().as_u16()
    ));
}
// ANCHOR_END: disconnect

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().state(ChatLog::default()).routes(routes![
        websocket_endpoint,
        item_websocket,
        chat,
        chat_log
    ])
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
    use thermite::{CloseCode, TestClient, TestWebSocketError};

    use super::*;

    #[tokio::test]
    async fn echoes_each_message() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let mut socket = client.websocket("/ws").connect().await?;

        socket.send_text("hello").await?;
        let answer = socket.receive_text().await?;
        socket.close(CloseCode::NORMAL, "").await;

        assert_eq!(answer, "Message text was: hello");
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn refuses_a_handshake_without_a_cookie_or_a_token() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let connected = client.websocket("/items/foo/ws").connect().await;

        let Err(TestWebSocketError::Denied(response)) = connected else {
            return Err("the handshake was accepted".into());
        };
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.text()?, r#"{"detail":"Not authenticated"}"#);
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these messages as they are
#[cfg(test)]
mod shown_tests {
    use std::time::Duration;

    use thermite::{CloseCode, TestClient, TestWebSocketError};

    use super::*;

    #[tokio::test]
    async fn reads_the_path_query_and_dependency_from_the_handshake() -> Result<(), Box<dyn Error>>
    {
        let client = TestClient::start(app()?).await?;
        let mut socket = client
            .websocket("/items/foo/ws?token=some-key-token&q=3")
            .connect()
            .await?;

        socket.send_text("hi").await?;
        let mut answers = Vec::new();
        for _ in 0..3 {
            answers.push(socket.receive_text().await?);
        }
        socket.close(CloseCode::NORMAL, "").await;

        assert_eq!(
            answers,
            [
                "Session cookie or query token value is: some-key-token",
                "Query parameter q is: 3",
                "Message text was: hi, for item ID: foo"
            ]
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn refuses_an_invalid_query_with_an_empty_403() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let connected = client
            .websocket("/items/foo/ws?token=some-key-token&q=three")
            .connect()
            .await;

        let Err(TestWebSocketError::Denied(response)) = connected else {
            return Err("the handshake was accepted".into());
        };
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.body().is_empty());
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn notes_when_a_client_leaves() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;
        let mut socket = client.websocket("/chat/7").connect().await?;

        socket.send_text("hello").await?;
        let answer = socket.receive_text().await?;
        socket.close(CloseCode::GOING_AWAY, "bye").await;
        // the handler notes the close after answering it, so the log may lag behind by a moment
        let mut lines: Vec<String> = Vec::new();
        for _ in 0..100 {
            lines = client.get("/chat/log").send().await?.json()?;
            if lines.len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(answer, "You wrote: hello");
        assert_eq!(
            lines,
            [
                "Client #7 says: hello",
                "Client #7 left the chat with the code 1001"
            ]
        );
        client.stop().await;
        Ok(())
    }
}
