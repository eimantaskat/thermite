// the thermite side of the comparison; app.py next to it is the same app in FastAPI
use std::env;
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{App, Json, Model, Path, Query, Server, get, post, routes};

#[derive(Serialize, Deserialize, Model)]
struct Item {
    name: String,
    price: f64,
    tags: Vec<String>,
}

#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: i64,
}

#[derive(Deserialize, Model)]
struct ItemQuery {
    q: Option<String>,
    #[model(default = 10)]
    limit: i64,
}

#[derive(Serialize, Model)]
struct ItemView {
    item_id: i64,
    q: Option<String>,
    limit: i64,
}

#[derive(Serialize, Model)]
struct Count {
    count: i64,
}

#[derive(Serialize, Model)]
struct Message {
    message: String,
}

#[get("/plaintext")]
async fn plaintext() -> &'static str {
    "Hello, World!"
}

#[get("/json")]
async fn json() -> Json<Message> {
    Json(Message {
        message: "Hello, World!".to_owned(),
    })
}

#[get("/items/{item_id}")]
async fn read_item(Path(path): Path<ItemPath>, Query(query): Query<ItemQuery>) -> Json<ItemView> {
    Json(ItemView {
        item_id: path.item_id,
        q: query.q,
        limit: query.limit,
    })
}

#[post("/items")]
async fn create_item(Json(item): Json<Item>) -> Json<Item> {
    Json(item)
}

#[post("/items/batch")]
async fn create_items(Json(items): Json<Vec<Item>>) -> Json<Count> {
    // only i64 has a schema, and no body holds more items than it counts
    let count = i64::try_from(items.len()).unwrap_or(i64::MAX);
    Json(Count { count })
}

// usage: fastapi_comparison <address> <worker threads>, where 1 runs everything on one thread
fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let address = arguments
        .next()
        .unwrap_or_else(|| "127.0.0.1:8000".to_owned());
    let threads: usize = arguments.next().map_or(Ok(1), |text| text.parse())?;
    let runtime = if threads <= 1 {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
    } else {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(threads)
            .enable_all()
            .build()?
    };
    runtime.block_on(async {
        let app = App::new().routes(routes![
            plaintext,
            json,
            read_item,
            create_item,
            create_items
        ])?;
        let server = Server::bind(address.as_str()).await?;
        eprintln!("listening on http://{}", server.local_address()?);
        server
            .serve_with_shutdown(app, || async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        Ok(())
    })
}
