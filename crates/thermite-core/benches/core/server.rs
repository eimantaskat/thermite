use std::hint::black_box;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use bytes::Bytes;
use criterion::{Criterion, Throughput, criterion_group};
use futures_util::{SinkExt, StreamExt};
use http::{Method, StatusCode};
use thermite_core::{Arguments, Request, Response, RouteDescription, Router, Server, WebSocket};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio_tungstenite::tungstenite;

use crate::http_client::{connect, handshake, open_stream, send};
use crate::support::{answer_ok, runtime};

const MEBIBYTE: usize = 1024 * 1024;
// over the 2 MiB default, and declared only, since the server answers before reading it
const TOO_LARGE: usize = 3 * MEBIBYTE;

async fn answer_body_length(request: Request, _: Arguments) -> Response {
    Response::new(request.body().len().to_string().into())
}

async fn echo_times(socket: WebSocket, times: usize) {
    let Ok(mut connection) = socket.accept().await else {
        return;
    };
    while let Ok(message) = connection.receive().await {
        for _ in 0..times {
            if connection.send(message.clone()).await.is_err() {
                return;
            }
        }
    }
}

async fn echo(_: Request, _: Arguments, socket: WebSocket) {
    echo_times(socket, 1).await;
}

// two writes before the next read, which Nagle's algorithm holds back until the first is acknowledged
async fn echo_twice(_: Request, _: Arguments, socket: WebSocket) {
    echo_times(socket, 2).await;
}

pub fn app() -> Router {
    let mut router = Router::new();
    router
        .route(RouteDescription::new(Method::GET, "/"), answer_ok)
        .expect("the root route");
    router
        .route(
            RouteDescription::new(Method::POST, "/upload"),
            answer_body_length,
        )
        .expect("the upload route");
    router
        .websocket(RouteDescription::new(Method::GET, "/ws"), echo)
        .expect("the WebSocket route");
    router
        .websocket(RouteDescription::new(Method::GET, "/ws/twice"), echo_twice)
        .expect("the twice echoing WebSocket route");
    router
}

// the server runs on the benchmark's runtime, which only polls it while a benchmark awaits
pub fn start(runtime: &Runtime) -> SocketAddr {
    runtime.block_on(async {
        let server = Server::bind("127.0.0.1:0").await.expect("a free port");
        let address = server.local_address().expect("the bound address");
        tokio::spawn(server.serve(app()));
        address
    })
}

// thousands of connections a second would run out of ports in TIME_WAIT, so each one is reset
fn close_at_once(stream: &TcpStream) {
    #[allow(deprecated)] // only deprecated for blocking on drop, which a zero timeout never does
    stream.set_linger(Some(Duration::ZERO)).expect("SO_LINGER");
}

async fn send_on_new_connection(address: SocketAddr) -> StatusCode {
    let stream = open_stream(address).await;
    close_at_once(&stream);
    send(
        &mut handshake(stream).await,
        Method::GET,
        "/",
        &Bytes::new(),
    )
    .await
}

fn upload_head(content_length: usize, connection: &str) -> String {
    format!(
        "POST /upload HTTP/1.1\r\nhost: localhost\r\nconnection: {connection}\r\ncontent-length: {content_length}\r\n\r\n"
    )
}

// sent on a connection of its own, which the server closes after answering
async fn send_head(address: SocketAddr, head: &str) -> String {
    let mut stream = open_stream(address).await;
    close_at_once(&stream);
    stream
        .write_all(head.as_bytes())
        .await
        .expect("the request head");
    // no shutdown of the write half, which would close gracefully and leave the client's port in TIME_WAIT
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("the response");
    String::from_utf8_lossy(&response).into_owned()
}

fn http(criterion: &mut Criterion) {
    let runtime = runtime();
    let address = start(&runtime);
    let empty = Bytes::new();
    let upload = Bytes::from(vec![b'a'; MEBIBYTE]);
    // the same exchange answered with a 200, so the two differ only in what the server does
    let closing = upload_head(0, "close");
    let too_large = upload_head(TOO_LARGE, "keep-alive");
    runtime.block_on(async {
        let mut sender = connect(address).await;
        assert_eq!(
            send(&mut sender, Method::GET, "/", &empty).await,
            StatusCode::OK
        );
        assert_eq!(
            send(&mut sender, Method::POST, "/upload", &upload).await,
            StatusCode::OK
        );
        assert_eq!(send_on_new_connection(address).await, StatusCode::OK);
        let answered = send_head(address, &closing).await;
        assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");
        let refused = send_head(address, &too_large).await;
        assert!(refused.starts_with("HTTP/1.1 413"), "{refused}");
    });

    let mut group = criterion.benchmark_group("server");
    group.bench_function("get/keep_alive", |bencher| {
        bencher.to_async(&runtime).iter_custom(|iterations| {
            let empty = &empty;
            async move {
                let mut sender = connect(address).await;
                let started = Instant::now();
                for _ in 0..iterations {
                    black_box(send(&mut sender, Method::GET, "/", empty).await);
                }
                started.elapsed()
            }
        });
    });
    group.bench_function("get/new_connection", |bencher| {
        bencher
            .to_async(&runtime)
            .iter(|| send_on_new_connection(address));
    });
    group.bench_function("post/connection_close", |bencher| {
        bencher
            .to_async(&runtime)
            .iter(|| send_head(address, &closing));
    });
    group.bench_function("body_too_large", |bencher| {
        bencher
            .to_async(&runtime)
            .iter(|| send_head(address, &too_large));
    });
    group.throughput(Throughput::Bytes(MEBIBYTE as u64));
    group.bench_function("body/1_mib", |bencher| {
        bencher.to_async(&runtime).iter_custom(|iterations| {
            let upload = &upload;
            async move {
                let mut sender = connect(address).await;
                let started = Instant::now();
                for _ in 0..iterations {
                    black_box(send(&mut sender, Method::POST, "/upload", upload).await);
                }
                started.elapsed()
            }
        });
    });
    group.finish();
}

async fn connect_websocket(
    address: SocketAddr,
    path: &str,
) -> tokio_tungstenite::WebSocketStream<TcpStream> {
    let (socket, _) = tokio_tungstenite::client_async(
        format!("ws://{address}{path}"),
        open_stream(address).await,
    )
    .await
    .expect("an accepted handshake");
    socket
}

fn websocket(criterion: &mut Criterion) {
    let runtime = runtime();
    let address = start(&runtime);
    let mut group = criterion.benchmark_group("websocket");
    let small = tungstenite::Message::text("hello");
    let cases = [
        ("echo/small", "/ws", small.clone(), 1),
        (
            "echo/1_mib",
            "/ws",
            tungstenite::Message::binary(Bytes::from(vec![b'a'; MEBIBYTE])),
            1,
        ),
        ("echo_twice/small", "/ws/twice", small, 2),
    ];
    for (name, path, message, replies) in cases {
        group.throughput(Throughput::Bytes(message.len() as u64));
        group.bench_function(name, |bencher| {
            bencher.to_async(&runtime).iter_custom(|iterations| {
                let message = &message;
                async move {
                    let mut socket = connect_websocket(address, path).await;
                    let started = Instant::now();
                    for _ in 0..iterations {
                        socket.send(message.clone()).await.expect("a sent message");
                        for _ in 0..replies {
                            let echoed = socket.next().await.expect("an open socket");
                            black_box(echoed.expect("an echoed message"));
                        }
                    }
                    started.elapsed()
                }
            });
        });
    }
    group.finish();
}

criterion_group!(benches, http, websocket);
