use std::net::SocketAddr;

use bytes::Bytes;
use http::{Method, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1::{self, SendRequest};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

pub async fn open_stream(address: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(address).await.expect("a connection");
    // a body written after its headers would otherwise wait for the server's delayed ACK
    stream.set_nodelay(true).expect("TCP_NODELAY");
    stream
}

pub async fn connect(address: SocketAddr) -> SendRequest<Full<Bytes>> {
    handshake(open_stream(address).await).await
}

pub async fn handshake(stream: TcpStream) -> SendRequest<Full<Bytes>> {
    let (sender, connection) = http1::handshake(TokioIo::new(stream))
        .await
        .expect("an HTTP/1.1 connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    sender
}

pub async fn send(
    sender: &mut SendRequest<Full<Bytes>>,
    method: Method,
    path: &str,
    body: &Bytes,
) -> StatusCode {
    let request = http::Request::builder()
        .method(method)
        .uri(path)
        .body(Full::new(body.clone()))
        .expect("a valid request");
    sender.ready().await.expect("an open connection");
    let response = sender
        .send_request(request)
        .await
        .expect("a response from the server");
    let status = response.status();
    response
        .into_body()
        .collect()
        .await
        .expect("the response body");
    status
}
