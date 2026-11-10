use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http::header::{CONTENT_TYPE, SEC_WEBSOCKET_PROTOCOL};
use http::{HeaderValue, Method, StatusCode, Version};
use thermite_core::{
    Arguments, BackgroundTasks, CloseCode, InProcessServer, InProcessWebSocket, Message, Parameter,
    Request, RequestHandler, Response, RouteDescription, RouteError, Router, Schema, WebSocket,
    WebSocketConnection, WebSocketDisconnect, WebSocketHandler, WebSocketHandshake,
    attach_background_tasks,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode as WireCode;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

use crate::support::{RunningServer, TEST_TIMEOUT, TestResult, text_response};

const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

fn handshake(path: &str) -> Request {
    http::Request::builder()
        .uri(path)
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", KEY)
        .body(Bytes::new())
        .unwrap_or_default()
}

fn router(handler: impl WebSocketHandler) -> Result<Router, RouteError> {
    let mut router = Router::new();
    router.websocket(RouteDescription::new(Method::GET, "/ws"), handler)?;
    Ok(router)
}

async fn connect(
    router: Router,
    request: Request,
) -> Result<InProcessWebSocket, Box<dyn std::error::Error>> {
    let server = InProcessServer::start(router).await?;
    match server.connect(request).await {
        WebSocketHandshake::Accepted(socket) => Ok(socket),
        WebSocketHandshake::Denied(response) => {
            Err(format!("the handshake was denied with {}", response.status()).into())
        }
    }
}

async fn denied(
    handler: impl RequestHandler,
    request: Request,
) -> Result<Response, Box<dyn std::error::Error>> {
    let server = InProcessServer::start(handler).await?.catch_panics();
    match server.connect(request).await {
        WebSocketHandshake::Accepted(_) => Err("the handshake was accepted".into()),
        WebSocketHandshake::Denied(response) => Ok(response),
    }
}

// a handler that panics is what some of these tests are about
#[allow(clippy::panic)]
fn fail(message: &'static str) {
    panic!("{message}");
}

async fn echo(_: Request, _: Arguments, socket: WebSocket) {
    let Ok(mut connection) = socket.accept().await else {
        return;
    };
    while let Ok(message) = connection.receive().await {
        if connection.send(message).await.is_err() {
            return;
        }
    }
}

#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<String>>>);

impl Seen {
    fn record(&self, event: impl Into<String>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.into());
    }

    fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[tokio::test]
async fn echoes_messages_through_an_accepted_socket() -> TestResult {
    let mut socket = connect(router(echo)?, handshake("/ws")).await?;

    socket.send(Message::Text("hello".to_owned())).await?;
    let text = socket.receive().await?;
    socket
        .send(Message::Binary(Bytes::from_static(b"\x00\x01")))
        .await?;
    let bytes = socket.receive_bytes().await?;

    assert_eq!(text, Message::Text("hello".to_owned()));
    assert_eq!(bytes, Bytes::from_static(b"\x00\x01"));
    Ok(())
}

#[tokio::test]
async fn the_handler_reads_the_code_and_reason_the_client_closed_with() -> TestResult {
    let (closed, closed_with) = oneshot::channel();
    let closed = Mutex::new(Some(closed));
    let handler = move |_: Request, _: Arguments, socket: WebSocket| {
        let closed = closed.lock().unwrap_or_else(PoisonError::into_inner).take();
        async move {
            let Ok(mut connection) = socket.accept().await else {
                return;
            };
            let first = connection.receive().await;
            let again = connection.send_text("too late").await;
            if let Some(closed) = closed {
                let _ = closed.send((first, again));
            }
        }
    };
    let mut socket = connect(router(handler)?, handshake("/ws")).await?;

    let disconnect = socket
        .close(CloseCode::new(4000).ok_or("code")?, "bye")
        .await;
    let (first, again) = timeout(TEST_TIMEOUT, closed_with).await??;

    let expected = first.err().ok_or("the socket is still open")?;
    assert_eq!(expected.code().as_u16(), 4000);
    assert_eq!(expected.reason(), "bye");
    assert_eq!(again, Err(expected));
    assert_eq!(disconnect.code().as_u16(), 4000);
    Ok(())
}

#[tokio::test]
async fn closes_what_a_returning_handler_left_open_with_1000() -> TestResult {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        if let Ok(mut connection) = socket.accept().await {
            let _ = connection.send_text("bye").await;
        }
    };
    let mut socket = connect(router(handler)?, handshake("/ws")).await?;

    let text = socket.receive_text().await?;
    let closed = socket
        .receive()
        .await
        .err()
        .ok_or("the socket is still open")?;

    assert_eq!(text, "bye");
    assert_eq!(closed.code(), CloseCode::NORMAL);
    assert_eq!(socket.receive().await, Err(closed.clone()));
    assert_eq!(
        socket.send(Message::Text("late".to_owned())).await,
        Err(closed.clone())
    );
    assert_eq!(socket.close(CloseCode::GOING_AWAY, "").await, closed);
    assert_eq!(socket.close(CloseCode::GOING_AWAY, "").await, closed);
    Ok(())
}

#[tokio::test]
async fn the_handler_closes_with_its_code_and_a_reason_cut_to_fit_a_frame() -> TestResult {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        if let Ok(mut connection) = socket.accept().await {
            let reason = format!("{}é", "x".repeat(122));
            connection.close(CloseCode::POLICY_VIOLATION, &reason).await;
        }
    };
    let mut socket = connect(router(handler)?, handshake("/ws")).await?;

    let closed = socket
        .receive()
        .await
        .err()
        .ok_or("the socket is still open")?;

    assert_eq!(closed.code(), CloseCode::POLICY_VIOLATION);
    assert_eq!(closed.reason(), "x".repeat(122));
    Ok(())
}

#[tokio::test]
async fn closing_with_no_status_sends_a_frame_without_a_code() -> TestResult {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        if let Ok(mut connection) = socket.accept().await {
            connection.close(CloseCode::NO_STATUS, "ignored").await;
        }
    };
    let mut socket = connect(router(handler)?, handshake("/ws")).await?;

    let closed = socket
        .receive()
        .await
        .err()
        .ok_or("the socket is still open")?;

    assert_eq!(closed.code(), CloseCode::NO_STATUS);
    assert_eq!(closed.reason(), "");
    Ok(())
}

#[tokio::test]
async fn a_message_of_the_wrong_kind_closes_with_1003() -> TestResult {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        if let Ok(mut connection) = socket.accept().await {
            let _ = connection.receive_text().await;
        }
    };
    let mut wants_text = connect(router(handler)?, handshake("/ws")).await?;
    let mut wants_bytes = connect(router(echo)?, handshake("/ws")).await?;

    wants_text
        .send(Message::Binary(Bytes::from_static(b"x")))
        .await?;
    wants_bytes.send(Message::Text("x".to_owned())).await?;

    let closed = wants_text
        .receive()
        .await
        .err()
        .ok_or("the socket is still open")?;
    assert_eq!(closed.code(), CloseCode::UNSUPPORTED_DATA);
    assert_eq!(closed.reason(), "expected a text message");
    let closed = wants_bytes
        .receive_bytes()
        .await
        .err()
        .ok_or("the socket is still open")?;
    assert_eq!(closed.code(), CloseCode::UNSUPPORTED_DATA);
    assert_eq!(closed.reason(), "expected a binary message");
    Ok(())
}

#[tokio::test]
async fn the_handler_reads_the_requested_subprotocols_and_picks_one() -> TestResult {
    let seen = Seen::default();
    let handler = {
        let seen = seen.clone();
        move |_: Request, _: Arguments, socket: WebSocket| {
            seen.record(socket.requested_subprotocols().join(" "));
            async move {
                let _ = socket
                    .accept_subprotocol(HeaderValue::from_static("chat"))
                    .await;
            }
        }
    };
    let mut request = handshake("/ws");
    request.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("graphql-ws, chat"),
    );

    let socket = connect(router(handler)?, request).await?;

    assert_eq!(socket.headers()[SEC_WEBSOCKET_PROTOCOL], "chat");
    assert_eq!(seen.recorded(), ["graphql-ws chat"]);
    assert!(format!("{socket:?}").starts_with("InProcessWebSocket { headers: "));
    Ok(())
}

#[tokio::test]
async fn a_denied_handshake_is_answered_with_the_response_once_the_handler_finishes() -> TestResult
{
    let seen = Seen::default();
    let handler = {
        let seen = seen.clone();
        move |_: Request, _: Arguments, socket: WebSocket| {
            let seen = seen.clone();
            async move {
                seen.record(format!("{socket:?}"));
                socket.deny(text_response(StatusCode::UNAUTHORIZED, "who are you"));
                tokio::task::yield_now().await;
                seen.record("after");
            }
        }
    };

    let response = denied(router(handler)?, handshake("/ws")).await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.body().as_bytes(),
        Some(&Bytes::from("who are you"))
    );
    assert_eq!(
        seen.recorded(),
        ["WebSocket { requested_subprotocols: [], .. }", "after"]
    );
    Ok(())
}

#[tokio::test]
async fn a_handler_that_denies_as_it_returns_is_answered_with_its_response() -> TestResult {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        socket.deny(text_response(StatusCode::FORBIDDEN, "no"));
    };

    let response = denied(router(handler)?, handshake("/ws")).await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(response.body().as_bytes(), Some(&Bytes::from("no")));
    Ok(())
}

#[tokio::test]
async fn a_handler_that_never_decides_the_handshake_answers_500_like_uvicorn() -> TestResult {
    let returns = |_: Request, _: Arguments, _: WebSocket| async {};
    let drops_the_socket = |_: Request, _: Arguments, socket: WebSocket| async move {
        drop(socket);
        tokio::task::yield_now().await;
    };
    let abandons_accept = |_: Request, _: Arguments, socket: WebSocket| async move {
        let mut accepting = std::pin::pin!(socket.accept());
        std::future::poll_fn(|cx| {
            let _ = accepting.as_mut().poll(cx);
            Poll::Ready(())
        })
        .await;
    };
    let panics = |_: Request, _: Arguments, _: WebSocket| async { fail("before accepting") };

    for router in [
        router(returns)?,
        router(drops_the_socket)?,
        router(abandons_accept)?,
        router(panics)?,
    ] {
        let response = denied(router, handshake("/ws")).await?;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.body().as_bytes(),
            Some(&Bytes::from("Internal Server Error"))
        );
    }
    Ok(())
}

#[tokio::test]
async fn whatever_keeps_a_websocket_from_being_accepted_answers_403_like_uvicorn() -> TestResult {
    let mut router = Router::new();
    router.websocket(
        RouteDescription::new(Method::GET, "/rooms/{room}")
            .parameter(Parameter::path("room", Schema::integer())),
        echo,
    )?;
    router.route(
        RouteDescription::new(Method::GET, "/http"),
        |_: Request, _: Arguments| async { text_response(StatusCode::OK, "http") },
    )?;
    let server = InProcessServer::start(router).await?;

    for path in ["/rooms/x", "/http", "/missing"] {
        let WebSocketHandshake::Denied(response) = server.connect(handshake(path)).await else {
            return Err(format!("{path} was accepted").into());
        };

        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
        assert!(response.body().as_bytes().is_some_and(Bytes::is_empty));
    }
    let mut post = handshake("/rooms/1");
    *post.method_mut() = Method::POST;
    let plain_get = http::Request::builder()
        .uri("/rooms/1")
        .body(Bytes::new())?;
    assert_eq!(server.send(post).await?.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        server.send(plain_get).await?.status(),
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[tokio::test]
async fn http_and_websocket_routes_share_a_path_but_not_a_table() -> TestResult {
    let mut router = Router::new();
    router.websocket(RouteDescription::new(Method::GET, "/chat"), echo)?;
    router.route(
        RouteDescription::new(Method::GET, "/chat"),
        |_: Request, _: Arguments| async { text_response(StatusCode::OK, "page") },
    )?;

    let conflict = router.websocket(RouteDescription::new(Method::GET, "/chat"), echo);
    let not_get = router.websocket(RouteDescription::new(Method::POST, "/other"), echo);
    let with_body = router.websocket(
        RouteDescription::new(Method::GET, "/body").body(Schema::object([])),
        echo,
    );
    let bad_prefix = router.websocket(
        RouteDescription::new(Method::GET, "/x").with_prefix("api"),
        echo,
    );

    assert!(matches!(conflict, Err(RouteError::Conflict { .. })));
    assert_eq!(
        not_get,
        Err(RouteError::WebSocketMethod {
            method: Method::POST,
            path: "/other".to_owned()
        })
    );
    assert_eq!(
        with_body,
        Err(RouteError::WebSocketBody {
            path: "/body".to_owned()
        })
    );
    assert!(matches!(bad_prefix, Err(RouteError::InvalidPrefix { .. })));
    assert_eq!(
        format!("{router:?}"),
        "Router { routes: [GET /chat], websockets: [GET /chat] }"
    );
    let server = InProcessServer::start(router).await?;
    let page = server
        .send(http::Request::builder().uri("/chat").body(Bytes::new())?)
        .await?;
    assert_eq!(page.body(), "page");
    assert!(matches!(
        server.connect(handshake("/chat")).await,
        WebSocketHandshake::Accepted(_)
    ));
    Ok(())
}

#[tokio::test]
#[should_panic(expected = "after accepting")]
async fn a_panic_after_accepting_reaches_the_test() {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        let _connection = socket.accept().await;
        fail("after accepting");
    };
    let Ok(router) = router(handler) else { return };
    let Ok(mut socket) = connect(router, handshake("/ws")).await else {
        return;
    };

    let _ = socket.receive().await;
}

#[tokio::test]
async fn a_caught_panic_after_accepting_closes_with_1011() -> TestResult {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        let _connection = socket.accept().await;
        fail("after accepting");
    };
    let server = InProcessServer::start(router(handler)?)
        .await?
        .catch_panics();
    let WebSocketHandshake::Accepted(mut socket) = server.connect(handshake("/ws")).await else {
        return Err("denied".into());
    };

    let closed = socket
        .receive()
        .await
        .err()
        .ok_or("the socket is still open")?;

    assert_eq!(closed.code(), CloseCode::INTERNAL_ERROR);
    Ok(())
}

#[tokio::test]
async fn a_client_that_goes_away_reads_as_abnormal_to_the_handler() -> TestResult {
    let (closed, closed_with) = oneshot::channel();
    let closed = Mutex::new(Some(closed));
    let handler = move |_: Request, _: Arguments, socket: WebSocket| {
        let closed = closed.lock().unwrap_or_else(PoisonError::into_inner).take();
        async move {
            let Ok(mut connection) = socket.accept().await else {
                return;
            };
            let received = connection.receive_text().await;
            if let Some(closed) = closed {
                let _ = closed.send(received);
            }
        }
    };
    let socket = connect(router(handler)?, handshake("/ws")).await?;

    drop(socket);
    let received = timeout(TEST_TIMEOUT, closed_with).await??;

    assert_eq!(
        received.err().ok_or("the socket is still open")?.code(),
        CloseCode::ABNORMAL
    );
    Ok(())
}

async fn open_over_tcp(
    server: &RunningServer,
) -> Result<tokio_tungstenite::WebSocketStream<TcpStream>, Box<dyn std::error::Error>> {
    let stream = TcpStream::connect(server.address).await?;
    let request = format!("ws://{}/ws", server.address).into_client_request()?;
    let (socket, response) = tokio_tungstenite::client_async(request, stream).await?;
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    Ok(socket)
}

async fn close_code(
    socket: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
) -> Result<Option<WireCode>, Box<dyn std::error::Error>> {
    let mut code = None;
    // reading on answers the close, which the server waits for before it lets go of the connection
    while let Some(received) = timeout(TEST_TIMEOUT, socket.next()).await? {
        if let tungstenite::Message::Close(frame) = received? {
            code = frame.map(|frame| frame.code);
        }
    }
    Ok(code)
}

#[tokio::test]
async fn echoes_over_a_real_connection_and_closes_when_the_client_does() -> TestResult {
    let server = RunningServer::start(router(echo)?).await?;
    let mut socket = open_over_tcp(&server).await?;

    socket
        .send(tungstenite::Message::text("over the wire"))
        .await?;
    let echoed = timeout(TEST_TIMEOUT, socket.next())
        .await?
        .ok_or("closed")??;
    socket
        .close(Some(CloseFrame {
            code: WireCode::Away,
            reason: "".into(),
        }))
        .await?;
    let answer = close_code(&mut socket).await?;

    assert_eq!(echoed, tungstenite::Message::text("over the wire"));
    assert_eq!(answer, Some(WireCode::Away));
    server.stop().await
}

// with Nagle's algorithm on, each second reply waits about 40 ms for Linux's delayed ack
#[tokio::test]
async fn a_second_write_before_a_read_is_not_held_back() -> TestResult {
    let echo_twice = |_: Request, _: Arguments, socket: WebSocket| async move {
        let Ok(mut connection) = socket.accept().await else {
            return;
        };
        while let Ok(message) = connection.receive().await {
            for _ in 0..2 {
                if connection.send(message.clone()).await.is_err() {
                    return;
                }
            }
        }
    };
    let server = RunningServer::start(router(echo_twice)?).await?;
    let mut socket = open_over_tcp(&server).await?;

    let started = Instant::now();
    for _ in 0..10 {
        socket.send(tungstenite::Message::text("twice")).await?;
        for _ in 0..2 {
            timeout(TEST_TIMEOUT, socket.next())
                .await?
                .ok_or("closed")??;
        }
    }
    let elapsed = started.elapsed();
    socket.close(None).await?;
    close_code(&mut socket).await?;

    assert!(elapsed < Duration::from_millis(200), "took {elapsed:?}");
    server.stop().await
}

#[tokio::test]
async fn shutting_down_closes_open_websockets_with_1012_like_uvicorn() -> TestResult {
    let server = RunningServer::start(router(echo)?).await?;
    let mut socket = open_over_tcp(&server).await?;

    server.shutdown.notify_one();
    let code = close_code(&mut socket).await?;

    assert_eq!(code, Some(WireCode::Restart));
    timeout(TEST_TIMEOUT, server.task).await???;
    Ok(())
}

#[tokio::test]
async fn a_panicking_handler_closes_with_1011_over_the_wire() -> TestResult {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        let _connection = socket.accept().await;
        fail("after accepting");
    };
    let server = RunningServer::start(router(handler)?).await?;
    let mut socket = open_over_tcp(&server).await?;

    let code = close_code(&mut socket).await?;

    assert_eq!(code, Some(WireCode::Error));
    server.stop().await
}

// the code of the close frame a frame written by hand is answered with
async fn close_after_raw_frame(frame: &[u8]) -> Result<[u8; 2], Box<dyn std::error::Error>> {
    let server = RunningServer::start(router(echo)?).await?;
    let mut stream = TcpStream::connect(server.address).await?;
    let request = format!(
        "GET /ws HTTP/1.1\r\nhost: x\r\nconnection: Upgrade\r\nupgrade: websocket\r\n\
         sec-websocket-version: 13\r\nsec-websocket-key: {KEY}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    stream.write_all(frame).await?;
    let mut received = Vec::new();
    let code = loop {
        let head_end = received.windows(4).position(|window| window == b"\r\n\r\n");
        if let Some([0x88, _, high, low, ..]) = head_end.and_then(|end| received.get(end + 4..)) {
            break [*high, *low];
        }
        let mut chunk = [0; 1024];
        let read = timeout(TEST_TIMEOUT, stream.read(&mut chunk)).await??;
        if read == 0 {
            return Err("no close frame".into());
        }
        received.extend_from_slice(&chunk[..read]);
    };
    drop(stream);
    server.stop().await?;
    Ok(code)
}

#[tokio::test]
async fn a_frame_over_16_mib_closes_with_1009_like_uvicorn() -> TestResult {
    let mut header = vec![0x82, 0x80 | 127];
    header.extend((16_u64 * 1024 * 1024 + 1).to_be_bytes());
    header.extend([1, 2, 3, 4]);

    let code = close_after_raw_frame(&header).await?;

    assert_eq!(code, 1009_u16.to_be_bytes());
    Ok(())
}

#[tokio::test]
async fn an_unmasked_frame_closes_with_1002() -> TestResult {
    let code = close_after_raw_frame(&[0x81, 0x02, b'h', b'i']).await?;

    assert_eq!(code, 1002_u16.to_be_bytes());
    Ok(())
}

#[tokio::test]
async fn text_that_is_not_utf8_closes_with_1007() -> TestResult {
    let mask = [1, 2, 3, 4];
    let payload: Vec<u8> = [0xff, 0xfe]
        .iter()
        .zip(mask.iter().cycle())
        .map(|(byte, mask)| byte ^ mask)
        .collect();
    let mut frame = vec![0x81, 0x80 | 2];
    frame.extend(mask);
    frame.extend(payload);

    let code = close_after_raw_frame(&frame).await?;

    assert_eq!(code, 1007_u16.to_be_bytes());
    Ok(())
}

// hyper drops an upgrade request whose client half-closes, so this reads without closing
async fn head_of_answer(
    server: &RunningServer,
    version: &str,
    headers: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(server.address).await?;
    let request = format!(
        "GET /ws {version}\r\nhost: x\r\nconnection: Upgrade\r\nupgrade: websocket\r\n{headers}\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let mut received = Vec::new();
    while !received.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut chunk = [0; 1024];
        let read = timeout(TEST_TIMEOUT, stream.read(&mut chunk)).await??;
        if read == 0 {
            break;
        }
        received.extend_from_slice(&chunk[..read]);
    }
    Ok(String::from_utf8(received)?)
}

#[tokio::test]
async fn answers_a_handshake_it_cannot_read_like_rfc_6455() -> TestResult {
    let server = RunningServer::start(router(echo)?).await?;

    let old_version = head_of_answer(
        &server,
        "HTTP/1.1",
        &format!("sec-websocket-version: 8\r\nsec-websocket-key: {KEY}\r\n"),
    )
    .await?;
    let no_key = head_of_answer(&server, "HTTP/1.1", "sec-websocket-version: 13\r\n").await?;

    assert!(
        old_version.starts_with("HTTP/1.1 426 Upgrade Required\r\n"),
        "{old_version}"
    );
    assert!(
        old_version.contains("sec-websocket-version: 13\r\n"),
        "{old_version}"
    );
    assert!(
        no_key.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{no_key}"
    );
    server.stop().await
}

#[tokio::test]
async fn a_ping_is_answered_and_never_reaches_the_handler() -> TestResult {
    let server = RunningServer::start(router(echo)?).await?;
    let mut socket = open_over_tcp(&server).await?;

    socket
        .send(tungstenite::Message::Ping("are you there".into()))
        .await?;
    socket
        .send(tungstenite::Message::text("after the ping"))
        .await?;
    let pong = timeout(TEST_TIMEOUT, socket.next())
        .await?
        .ok_or("closed")??;
    let echoed = timeout(TEST_TIMEOUT, socket.next())
        .await?
        .ok_or("closed")??;

    socket.close(None).await?;
    close_code(&mut socket).await?;

    assert_eq!(pong, tungstenite::Message::Pong("are you there".into()));
    assert_eq!(echoed, tungstenite::Message::text("after the ping"));
    server.stop().await
}

// the handler waits for the client to be gone, then does what it is given
async fn after_the_client_left<Done: Future<Output = WebSocketDisconnect> + Send + 'static>(
    then: fn(WebSocketConnection) -> Done,
) -> Result<(String, WebSocketDisconnect), Box<dyn std::error::Error>> {
    let (gone, wait_until_gone) = oneshot::channel::<()>();
    let (done, outcome) = oneshot::channel();
    let slots = Mutex::new(Some((wait_until_gone, done)));
    let handler = move |_: Request, _: Arguments, socket: WebSocket| {
        let slots = slots.lock().unwrap_or_else(PoisonError::into_inner).take();
        async move {
            let (Some((wait_until_gone, done)), Ok(connection)) = (slots, socket.accept().await)
            else {
                return;
            };
            let shown = format!("{connection:?}");
            let _ = wait_until_gone.await;
            let _ = done.send((shown, then(connection).await));
        }
    };
    let socket = connect(router(handler)?, handshake("/ws")).await?;

    drop(socket);
    let _ = gone.send(());
    Ok(timeout(TEST_TIMEOUT, outcome).await??)
}

#[tokio::test]
async fn sending_to_a_client_that_went_away_reads_as_abnormal() -> TestResult {
    let (shown, ended) = after_the_client_left(|mut connection| async move {
        loop {
            if let Err(ended) = connection.send_text("anyone?").await {
                return ended;
            }
        }
    })
    .await?;

    assert_eq!(ended.code(), CloseCode::ABNORMAL);
    assert_eq!(shown, "WebSocketConnection { .. }");
    Ok(())
}

#[tokio::test]
async fn closing_after_the_client_went_away_keeps_the_code_it_closed_with() -> TestResult {
    let (_, ended) = after_the_client_left(|mut connection| async move {
        connection.close(CloseCode::GOING_AWAY, "bye").await
    })
    .await?;

    assert_eq!(
        (ended.code(), ended.reason()),
        (CloseCode::GOING_AWAY, "bye")
    );
    Ok(())
}

#[tokio::test]
async fn a_101_whose_status_was_changed_drops_the_session() -> TestResult {
    let router = Arc::new(router(echo)?);
    let changes_the_status = move |request: Request| {
        let router = Arc::clone(&router);
        async move {
            let mut response = router.handle(request).await;
            *response.status_mut() = StatusCode::OK;
            response
        }
    };
    let server = InProcessServer::start(changes_the_status).await?;

    let handshake = server.connect(handshake("/ws")).await;

    let WebSocketHandshake::Denied(response) = handshake else {
        return Err("the session survived".into());
    };
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

// denies with a response that has a background task, then goes on
fn denies_then_goes_on(seen: &Seen) -> impl WebSocketHandler {
    let seen = seen.clone();
    move |_: Request, _: Arguments, socket: WebSocket| {
        let seen = seen.clone();
        async move {
            let mut response = text_response(StatusCode::UNAUTHORIZED, "no");
            let tasks = BackgroundTasks::new();
            let in_task = seen.clone();
            tasks.add(async move { in_task.record("background task") });
            attach_background_tasks(&mut response, tasks);
            socket.deny(response);
            tokio::task::yield_now().await;
            seen.record("after denying");
        }
    }
}

#[tokio::test]
async fn a_denial_keeps_the_background_tasks_of_its_response() -> TestResult {
    let seen = Seen::default();

    let response = denied(router(denies_then_goes_on(&seen))?, handshake("/ws")).await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(seen.recorded(), ["after denying", "background task"]);
    Ok(())
}

#[tokio::test]
async fn what_a_handler_does_after_denying_survives_its_response_being_replaced() -> TestResult {
    let seen = Seen::default();
    let router = Arc::new(router(denies_then_goes_on(&seen))?);
    // as an exception handler does, answering with a response of its own
    let replaces = move |request: Request| {
        let router = Arc::clone(&router);
        async move {
            let denial = router.handle(request).await;
            text_response(denial.status(), "replaced")
        }
    };

    let response = denied(replaces, handshake("/ws")).await?;

    assert_eq!(response.body().as_bytes(), Some(&Bytes::from("replaced")));
    assert_eq!(seen.recorded(), ["after denying"]);
    Ok(())
}

async fn sends_until_closed(_: Request, _: Arguments, socket: WebSocket) {
    let Ok(mut connection) = socket.accept().await else {
        return;
    };
    while connection.send_text("tick").await.is_ok() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn shutting_down_closes_a_websocket_whose_handler_only_sends() -> TestResult {
    let server = RunningServer::start(router(sends_until_closed)?).await?;
    let mut socket = open_over_tcp(&server).await?;
    timeout(TEST_TIMEOUT, socket.next())
        .await?
        .ok_or("closed")??;

    server.shutdown.notify_one();
    let code = close_code(&mut socket).await?;

    assert_eq!(code, Some(WireCode::Restart));
    timeout(TEST_TIMEOUT, server.task).await???;
    Ok(())
}

#[tokio::test]
async fn shutting_down_closes_a_websocket_whose_handler_waits_on_something_else() -> TestResult {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        let _connection = socket.accept().await;
        std::future::pending::<()>().await;
    };
    let server = RunningServer::start_configured(
        |server| server.with_shutdown_timeout(Duration::from_millis(200)),
        router(handler)?,
    )
    .await?;
    let mut socket = open_over_tcp(&server).await?;

    server.shutdown.notify_one();
    let code = close_code(&mut socket).await?;

    assert_eq!(code, Some(WireCode::Restart));
    timeout(TEST_TIMEOUT, server.task).await???;
    Ok(())
}

async fn panics_after_accepting(_: Request, _: Arguments, socket: WebSocket) {
    let _connection = socket.accept().await;
    fail("after accepting");
}

#[tokio::test]
#[should_panic(expected = "after accepting")]
async fn a_panic_after_accepting_reaches_a_test_that_only_sends() {
    let Ok(router) = router(panics_after_accepting) else {
        return;
    };
    let Ok(mut socket) = connect(router, handshake("/ws")).await else {
        return;
    };

    for _ in 0..100 {
        let _ = socket.send(Message::Text("hello".to_owned())).await;
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
#[should_panic(expected = "after accepting")]
async fn a_panic_after_accepting_reaches_a_test_that_only_closes() {
    let Ok(router) = router(panics_after_accepting) else {
        return;
    };
    let Ok(mut socket) = connect(router, handshake("/ws")).await else {
        return;
    };

    socket.close(CloseCode::NORMAL, "").await;
}

#[tokio::test]
#[should_panic(expected = "after accepting")]
async fn a_panic_before_the_test_drops_the_socket_reaches_it() {
    let (panicking, about_to_panic) = oneshot::channel();
    let panicking = Mutex::new(Some(panicking));
    let handler = move |_: Request, _: Arguments, socket: WebSocket| {
        let panicking = panicking
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        async move {
            let _connection = socket.accept().await;
            if let Some(panicking) = panicking {
                let _ = panicking.send(());
            }
            fail("after accepting");
        }
    };
    let Ok(router) = router(handler) else { return };
    let Ok(socket) = connect(router, handshake("/ws")).await else {
        return;
    };

    // a test runs on one thread, so the handler has panicked by the time this wakes
    let _ = about_to_panic.await;
    drop(socket);
}

#[tokio::test]
async fn a_handshake_over_the_body_limit_is_answered_413_like_the_server() -> TestResult {
    let server = InProcessServer::start(router(echo)?)
        .await?
        .with_body_limit(1);
    let mut request = handshake("/ws");
    *request.body_mut() = Bytes::from_static(b"too long");

    let WebSocketHandshake::Denied(response) = server.connect(request).await else {
        return Err("the handshake was accepted".into());
    };

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    Ok(())
}

#[tokio::test]
async fn refuses_an_http_1_0_handshake_with_400_since_it_cannot_upgrade() -> TestResult {
    let server = RunningServer::start(router(echo)?).await?;
    let mut request = handshake("/ws");
    *request.version_mut() = Version::HTTP_10;

    let over_the_wire = head_of_answer(
        &server,
        "HTTP/1.0",
        &format!("sec-websocket-version: 13\r\nsec-websocket-key: {KEY}\r\n"),
    )
    .await?;
    let in_process = denied(router(echo)?, request).await?;

    assert!(
        over_the_wire.starts_with("HTTP/1.0 400 Bad Request\r\n"),
        "{over_the_wire}"
    );
    assert_eq!(in_process.status(), StatusCode::BAD_REQUEST);
    server.stop().await
}

#[tokio::test]
#[should_panic(expected = "after a message")]
async fn a_panic_reaches_the_test_after_a_message_was_received() {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        if let Ok(mut connection) = socket.accept().await {
            let _ = connection.send_text("first").await;
        }
        fail("after a message");
    };
    let Ok(router) = router(handler) else { return };
    let Ok(mut socket) = connect(router, handshake("/ws")).await else {
        return;
    };

    let _ = socket.receive().await;
    let _ = socket.receive().await;
}

#[tokio::test(start_paused = true)]
async fn closing_stops_waiting_for_a_handler_that_never_ends() -> TestResult {
    let handler = |_: Request, _: Arguments, socket: WebSocket| async move {
        let _connection = socket.accept().await;
        std::future::pending::<()>().await;
    };
    let mut socket = connect(router(handler)?, handshake("/ws")).await?;

    let closed = timeout(TEST_TIMEOUT * 4, socket.close(CloseCode::NORMAL, "")).await?;

    assert_eq!(closed.code(), CloseCode::NORMAL);
    Ok(())
}
