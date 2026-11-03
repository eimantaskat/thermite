// dependencies are written here the way users write them, as async fns that may not await
#![allow(clippy::unused_async_trait_impl)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thermite::http::HeaderValue;
use thermite::http::header::{CONTENT_TYPE, SEC_WEBSOCKET_PROTOCOL, WWW_AUTHENTICATE};
use thermite::{
    App, Bytes, CloseCode, Dependency, Depends, HttpError, Json, Message, Model, Next, Path, Query,
    Request, Route, RouteError, Router, SecurityScheme, ServerError, State, StatusCode, TestClient,
    TestResponse, TestWebSocketError, WebSocket, WebSocketDisconnect, routes, websocket,
};

use tokio::sync::oneshot;
use tokio::time::timeout;

use crate::support::TestResult;

#[derive(Deserialize, Model)]
struct Room {
    room: String,
}

#[derive(Deserialize, Model)]
struct Paging {
    limit: i64,
}

#[derive(Serialize, Deserialize, Model, Debug, PartialEq)]
struct Item {
    name: String,
    price: f64,
}

#[derive(SecurityScheme)]
#[security(http_bearer)]
struct Bearer(thermite::HttpAuthorizationCredentials);

async fn chat(
    socket: WebSocket,
    Path(room): Path<Room>,
    Query(paging): Query<Paging>,
) -> Result<(), WebSocketDisconnect> {
    let mut connection = socket.accept().await?;
    loop {
        let text = connection.receive_text().await?;
        connection
            .send_text(format!("{}/{}: {text}", room.room, paging.limit))
            .await?;
    }
}

// answers each kind of message with the same kind, through every way the connection has to send
async fn mirror(socket: WebSocket) -> Result<(), WebSocketDisconnect> {
    let mut connection = socket.accept().await?;
    let item: Item = connection.receive_json().await?;
    connection
        .send_json(&Item {
            price: item.price * 2.0,
            ..item
        })
        .await?;
    let bytes = connection.receive_bytes().await?;
    connection.send_bytes(bytes).await?;
    let message = connection.receive().await?;
    connection.send(message).await?;
    let closed = connection.close(CloseCode::GOING_AWAY, "done").await;
    Err(closed)
}

fn denied(
    connected: Result<thermite::TestWebSocket, TestWebSocketError>,
) -> Result<TestResponse, Box<dyn std::error::Error>> {
    match connected {
        Err(TestWebSocketError::Denied(response)) => Ok(*response),
        Err(error) => Err(error.into()),
        Ok(_) => Err("the handshake was accepted".into()),
    }
}

// a handler that panics is what some of these tests are about
#[allow(clippy::panic)]
fn fail(message: &'static str) {
    panic!("{message}");
}

#[tokio::test]
async fn reads_path_and_query_values_from_the_handshake() -> TestResult {
    let client = TestClient::start(App::new().websocket("/rooms/{room}", chat)?).await?;
    let mut socket = client.websocket("/rooms/lobby?limit=3").connect().await?;

    socket.send_text("hi").await?;
    let answer = socket.receive_text().await?;
    let closed = socket.close(CloseCode::NORMAL, "").await;

    assert_eq!(answer, "lobby/3: hi");
    assert_eq!(closed.code(), CloseCode::NORMAL);
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn sends_and_receives_json_bytes_and_messages() -> TestResult {
    let (ended, ended_with) = oneshot::channel();
    let ended = Mutex::new(Some(ended));
    let reports_the_end = move |socket: WebSocket| {
        let ended = ended.lock().unwrap_or_else(PoisonError::into_inner).take();
        async move {
            let outcome = mirror(socket).await;
            if let Some(ended) = ended {
                let _ = ended.send(outcome);
            }
        }
    };
    let client = TestClient::start(App::new().websocket("/mirror", reports_the_end)?).await?;
    let mut socket = client.websocket("/mirror").connect().await?;

    socket
        .send_json(&json!({"name": "pen", "price": 1.5}))
        .await?;
    let item: Item = socket.receive_json().await?;
    socket.send_bytes(Bytes::from_static(b"\x01\x02")).await?;
    let bytes = socket.receive_bytes().await?;
    socket.send(Message::Text("again".to_owned())).await?;
    let message = socket.receive().await?;
    let closed = socket
        .receive()
        .await
        .err()
        .ok_or("the socket is still open")?;
    let outcome = timeout(Duration::from_secs(5), ended_with).await??;

    assert_eq!(outcome, Err(closed.clone()));
    assert_eq!(
        item,
        Item {
            name: "pen".to_owned(),
            price: 3.0
        }
    );
    assert_eq!(bytes, Bytes::from_static(b"\x01\x02"));
    assert_eq!(message, Message::Text("again".to_owned()));
    assert_eq!(
        (closed.code(), closed.reason()),
        (CloseCode::GOING_AWAY, "done")
    );
    Ok(())
}

#[tokio::test]
async fn json_that_does_not_read_closes_with_1007() -> TestResult {
    let client = TestClient::start(App::new().websocket("/mirror", mirror)?).await?;
    let mut sends_text = client.websocket("/mirror").connect().await?;
    let echo = |socket: WebSocket| async move {
        let mut connection = socket.accept().await?;
        connection.send_text("not json").await?;
        connection.receive().await.map(drop)
    };
    let client_reading = TestClient::start(App::new().websocket("/echo", echo)?).await?;
    let mut reads_text = client_reading.websocket("/echo").connect().await?;

    sends_text.send_text("{\"name\": 1}").await?;
    let server_closed = sends_text
        .receive()
        .await
        .err()
        .ok_or("the socket is still open")?;
    let client_closed = reads_text
        .receive_json::<Item>()
        .await
        .err()
        .ok_or("the socket is still open")?;

    assert_eq!(server_closed.code(), CloseCode::INVALID_DATA);
    assert!(
        server_closed.reason().starts_with("invalid type"),
        "{}",
        server_closed.reason()
    );
    assert_eq!(client_closed.code(), CloseCode::INVALID_DATA);
    Ok(())
}

#[tokio::test]
async fn a_value_serde_cannot_write_closes_with_1011() -> TestResult {
    let unwritable: HashMap<(i32, i32), i32> = HashMap::from([((1, 2), 3)]);
    let handler = move |socket: WebSocket| {
        let unwritable = unwritable.clone();
        async move {
            let mut connection = socket.accept().await?;
            connection.send_json(&unwritable).await
        }
    };
    let client = TestClient::start(App::new().websocket("/unwritable", handler)?).await?;
    let mut socket = client.websocket("/unwritable").connect().await?;

    let server_closed = socket
        .receive()
        .await
        .err()
        .ok_or("the socket is still open")?;
    let client_closed = socket
        .send_json(&HashMap::from([((1, 2), 3)]))
        .await
        .err()
        .ok_or("the socket is still open")?;

    assert_eq!(server_closed.code(), CloseCode::INTERNAL_ERROR);
    assert_eq!(client_closed, server_closed);
    Ok(())
}

#[tokio::test]
async fn refuses_what_fastapi_refuses_with_403_and_plain_requests_with_404() -> TestResult {
    let client = TestClient::start(App::new().websocket("/rooms/{room}", chat)?).await?;

    let invalid = denied(client.websocket("/rooms/lobby?limit=x").connect().await)?;
    let missing = denied(client.websocket("/missing").connect().await)?;
    let docs = denied(client.websocket("/docs").connect().await)?;
    let plain = client.get("/rooms/lobby?limit=3").send().await?;

    for response in [&invalid, &missing, &docs] {
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
        assert!(response.body().is_empty());
    }
    assert_eq!(plain.status(), StatusCode::NOT_FOUND);
    assert_eq!(client.get("/docs").send().await?.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn a_rejection_before_accepting_is_sent_as_the_response_like_an_http_exception() -> TestResult
{
    let needs_token = |socket: WebSocket, Bearer(token): Bearer| async move {
        let mut connection = socket.accept().await?;
        connection.send_text(token.credentials).await
    };
    let denies = |socket: WebSocket| async move {
        socket.deny(HttpError::new(StatusCode::UNAUTHORIZED, "who"));
    };
    let app = App::new()
        .websocket("/token", needs_token)?
        .websocket("/denies", denies)?;
    let client = TestClient::start(app).await?;

    let missing_token = denied(client.websocket("/token").connect().await)?;
    let denial = denied(client.websocket("/denies").connect().await)?;
    let mut with_token = client
        .websocket("/token")
        .header("authorization", "Bearer abc")
        .connect()
        .await?;

    assert_eq!(missing_token.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(missing_token.headers()[WWW_AUTHENTICATE], "Bearer");
    assert_eq!(
        missing_token.json::<Value>()?,
        json!({"detail": "Not authenticated"})
    );
    assert_eq!(denial.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(denial.json::<Value>()?, json!({"detail": "who"}));
    assert_eq!(with_token.receive_text().await?, "abc");
    Ok(())
}

#[tokio::test]
async fn exception_handlers_apply_to_a_denial_but_middleware_never_sees_a_handshake() -> TestResult
{
    let denies = |socket: WebSocket| async move {
        socket.deny(HttpError::new(StatusCode::UNAUTHORIZED, "who"));
    };
    let accepts = |socket: WebSocket| async move { socket.accept().await.map(drop) };
    let app = App::new()
        .middleware(|request: Request, next: Next| async move {
            let mut response = next.run(request).await;
            response
                .headers_mut()
                .insert("x-middleware", HeaderValue::from_static("yes"));
            response
        })
        .exception_handler(|error: &HttpError| {
            (error.status(), Json(json!({"custom": error.detail()})))
        })
        .websocket("/denies", denies)?
        .websocket("/accepts", accepts)?
        .get("/page", || async { "page" })?;
    let client = TestClient::start(app).await?;

    let denial = denied(client.websocket("/denies").connect().await)?;
    let accepted = client.websocket("/accepts").connect().await?;
    let page = client.get("/page").send().await?;

    assert_eq!(denial.json::<Value>()?, json!({"custom": "who"}));
    assert!(!denial.headers().contains_key("x-middleware"));
    assert!(!accepted.headers().contains_key("x-middleware"));
    assert_eq!(page.headers()["x-middleware"], "yes");
    Ok(())
}

#[tokio::test]
async fn a_handler_that_returns_without_accepting_answers_500_like_uvicorn() -> TestResult {
    let returns = |_: WebSocket| async {};
    let client = TestClient::start(App::new().websocket("/returns", returns)?).await?;

    let response = denied(client.websocket("/returns").connect().await)?;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    Ok(())
}

#[tokio::test]
#[should_panic(expected = "the handler's own message")]
async fn a_panic_in_the_handler_fails_the_test() {
    let panics = |socket: WebSocket| async move {
        let _connection = socket.accept().await;
        fail("the handler's own message");
    };
    let Ok(app) = App::new().websocket("/panics", panics) else {
        return;
    };
    let Ok(client) = TestClient::start(app).await else {
        return;
    };
    let Ok(mut socket) = client.websocket("/panics").connect().await else {
        return;
    };

    let _ = socket.receive_text().await;
}

#[tokio::test]
async fn a_client_catching_panics_sees_1011_after_accepting_and_500_before() -> TestResult {
    let after = |socket: WebSocket| async move {
        let _connection = socket.accept().await;
        fail("after accepting");
    };
    let before = |_: WebSocket| async { fail("before accepting") };
    let app = App::new()
        .websocket("/after", after)?
        .websocket("/before", before)?;
    let client = TestClient::start(app).await?.catch_panics();

    let mut socket = client.websocket("/after").connect().await?;
    let closed = socket
        .receive_bytes()
        .await
        .err()
        .ok_or("the socket is still open")?;
    let response = denied(client.websocket("/before").connect().await)?;

    assert_eq!(closed.code(), CloseCode::INTERNAL_ERROR);
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    Ok(())
}

#[tokio::test]
async fn accepts_one_of_the_requested_subprotocols() -> TestResult {
    let picks = |socket: WebSocket| async move {
        let picked = socket
            .requested_subprotocols()
            .last()
            .cloned()
            .unwrap_or_default();
        let mut connection = socket
            .accept_subprotocol(
                HeaderValue::from_str(&picked).unwrap_or(HeaderValue::from_static("none")),
            )
            .await?;
        connection.send_text(picked).await
    };
    let client = TestClient::start(App::new().websocket("/picks", picks)?).await?;

    let mut socket = client
        .websocket("/picks")
        .header(SEC_WEBSOCKET_PROTOCOL, "graphql-ws, chat")
        .connect()
        .await?;

    assert_eq!(socket.headers()[SEC_WEBSOCKET_PROTOCOL], "chat");
    assert_eq!(socket.receive_text().await?, "chat");
    assert!(format!("{socket:?}").starts_with("TestWebSocket(InProcessWebSocket { headers: "));
    Ok(())
}

#[derive(Clone)]
struct Greeting(String);

impl Dependency for Greeting {
    type Needs = Query<Room>;
    type Rejection = Infallible;

    async fn resolve(Query(room): Query<Room>) -> Result<Self, Infallible> {
        Ok(Self(format!("welcome to {}", room.room)))
    }
}

#[websocket("/greet")]
async fn greet(
    socket: WebSocket,
    Depends(greeting): Depends<Greeting>,
    State(suffix): State<&'static str>,
) -> Result<(), WebSocketDisconnect> {
    let mut connection = socket.accept().await?;
    connection
        .send_text(format!("{}{suffix}", greeting.0))
        .await
}

#[tokio::test]
async fn routers_attributes_dependencies_and_state_work_as_for_http_routes() -> TestResult {
    let router = Router::new()
        .prefix("/api")
        .websocket("/rooms/{room}", chat)
        .routes(routes![greet]);
    let app = App::new().state("!").include_router(router)?;
    let client = TestClient::start(app).await?;

    let mut chat_socket = client
        .websocket("/api/rooms/lobby?limit=1")
        .connect()
        .await?;
    let mut greet_socket = client.websocket("/api/greet?room=hall").connect().await?;
    chat_socket.send_text("hi").await?;

    assert_eq!(chat_socket.receive_text().await?, "lobby/1: hi");
    assert_eq!(greet_socket.receive_text().await?, "welcome to hall!");
    assert_eq!(format!("{:?}", greet::route()), "WEBSOCKET /greet");
    Ok(())
}

#[tokio::test]
async fn the_docs_leave_websocket_routes_out_like_fastapi() -> TestResult {
    let app = App::new()
        .websocket("/rooms/{room}", chat)?
        .get("/page", || async { "page" })?;
    let client = TestClient::start(app).await?;

    let document: Value = client.get("/openapi.json").send().await?.json()?;

    assert_eq!(
        document["paths"].as_object().map(serde_json::Map::len),
        Some(1)
    );
    assert!(document["paths"].get("/page").is_some());
    Ok(())
}

#[tokio::test]
async fn a_websocket_route_names_itself_when_its_state_is_missing() -> TestResult {
    let app = App::new().websocket("/greet", greet)?;

    let Err(ServerError::Startup(error)) = TestClient::start(app).await else {
        return Err("the app started".into());
    };

    assert!(
        error
            .to_string()
            .starts_with("WEBSOCKET /greet needs the state"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
async fn a_websocket_route_cannot_read_a_body() -> TestResult {
    let reads_body =
        |socket: WebSocket, _: Json<Item>| async move { socket.accept().await.map(drop) };

    let refused = App::new().websocket("/body", reads_body);

    assert_eq!(
        refused.err(),
        Some(RouteError::WebSocketBody {
            path: "/body".to_owned()
        })
    );
    assert_eq!(
        format!("{:?}", Route::websocket("/body", reads_body)),
        "WEBSOCKET /body"
    );
    Ok(())
}

#[tokio::test]
async fn a_client_dropping_the_socket_ends_the_handler_with_a_disconnect() -> TestResult {
    let seen: Arc<Mutex<Option<WebSocketDisconnect>>> = Arc::default();
    let handler = {
        let seen = Arc::clone(&seen);
        move |socket: WebSocket| {
            let seen = Arc::clone(&seen);
            async move {
                let mut connection = socket.accept().await?;
                connection.send_text("ready").await?;
                let Err(ended) = connection.receive().await else {
                    return Ok(());
                };
                *seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(ended.clone());
                Err(ended)
            }
        }
    };
    let client = TestClient::start(App::new().websocket("/drop", handler)?).await?;
    let mut socket = client.websocket("/drop").connect().await?;

    socket.receive_text().await?;
    drop(socket);
    for _ in 0..100 {
        if seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
        {
            break;
        }
        tokio::task::yield_now().await;
    }

    let ended = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(ended.map(|ended| ended.code()), Some(CloseCode::ABNORMAL));
    Ok(())
}

#[tokio::test]
async fn a_bad_path_fails_before_the_handshake() -> TestResult {
    let client = TestClient::start(App::new()).await?;

    let error = client
        .websocket("no-slash")
        .connect()
        .await
        .err()
        .ok_or("the socket is still open")?;

    assert!(matches!(error, TestWebSocketError::Request(_)));
    assert_eq!(error.to_string(), "the path `no-slash` must start with `/`");
    let denial = denied(client.websocket("/x").connect().await)?;
    assert_eq!(
        TestWebSocketError::Denied(Box::new(denial)).to_string(),
        "the handshake was answered with 403 Forbidden"
    );
    Ok(())
}

#[tokio::test]
async fn what_a_handler_does_after_denying_runs_when_an_exception_handler_answers() -> TestResult {
    let (finished, finished_with) = oneshot::channel();
    let finished = Mutex::new(Some(finished));
    let denies = move |socket: WebSocket| {
        let finished = finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        async move {
            socket.deny(HttpError::new(StatusCode::UNAUTHORIZED, "who"));
            tokio::task::yield_now().await;
            if let Some(finished) = finished {
                let _ = finished.send("cleaned up");
            }
        }
    };
    let app = App::new()
        .exception_handler(|error: &HttpError| {
            (error.status(), Json(json!({"custom": error.detail()})))
        })
        .websocket("/denies", denies)?;
    let client = TestClient::start(app).await?;

    let denial = denied(client.websocket("/denies").connect().await)?;

    assert_eq!(denial.json::<Value>()?, json!({"custom": "who"}));
    assert_eq!(finished_with.await?, "cleaned up");
    Ok(())
}
