use thermite::{WebSocket, websocket};

#[websocket("/chat")]
async fn chat(socket: WebSocket) -> i64 {
    drop(socket);
    1
}

fn main() {}
