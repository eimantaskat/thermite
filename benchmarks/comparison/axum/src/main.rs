// the axum side of the comparison; thermite.rs and fastapi_app.py beside it are the same app
mod validation;

use std::env;
use std::error::Error;

use axum::Router;
use axum::extract::{Path, Query};
use axum::routing::{get, post};
use axum::serve::ListenerExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;

use validation::{Location, Validate, Validated, Validator};

#[derive(Serialize, Deserialize)]
struct Item {
    name: String,
    price: f64,
    tags: Vec<String>,
}

impl Validate for Item {
    fn validate(validator: &mut Validator, value: &Value, location: &mut Location) -> Option<Self> {
        validator.object(value, location)?;
        let name = validator.field(value, location, "name", Validator::string);
        let price = validator.field(value, location, "price", Validator::float);
        let tags = validator.field(value, location, "tags", |validator, tags, location| {
            validator.list(tags, location, Validator::string)
        });
        Some(Self {
            name: name?,
            price: price?,
            tags: tags?,
        })
    }
}

#[derive(Deserialize)]
struct ItemPath {
    item_id: i64,
}

#[derive(Deserialize)]
struct ItemQuery {
    q: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    10
}

#[derive(Serialize)]
struct ItemView {
    item_id: i64,
    q: Option<String>,
    limit: i64,
}

#[derive(Serialize)]
struct Count {
    count: usize,
}

#[derive(Serialize)]
struct Message {
    message: String,
}

async fn plaintext() -> &'static str {
    "Hello, World!"
}

async fn json() -> axum::Json<Message> {
    axum::Json(Message {
        message: "Hello, World!".to_owned(),
    })
}

async fn read_item(
    Path(path): Path<ItemPath>,
    Query(query): Query<ItemQuery>,
) -> axum::Json<ItemView> {
    axum::Json(ItemView {
        item_id: path.item_id,
        q: query.q,
        limit: query.limit,
    })
}

async fn create_item(Validated(item): Validated<Item>) -> axum::Json<Item> {
    axum::Json(item)
}

async fn create_items(Validated(items): Validated<Vec<Item>>) -> axum::Json<Count> {
    axum::Json(Count { count: items.len() })
}

// usage: axum-comparison <address> <worker threads>, where 1 runs everything on one thread
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
        let app = Router::new()
            .route("/plaintext", get(plaintext))
            .route("/json", get(json))
            .route("/items/{item_id}", get(read_item))
            .route("/items", post(create_item))
            .route("/items/batch", post(create_items));
        let listener = TcpListener::bind(address.as_str()).await?;
        eprintln!("listening on http://{}", listener.local_addr()?);
        // thermite and uvicorn both turn Nagle's algorithm off
        let listener = listener.tap_io(|stream| {
            let _ = stream.set_nodelay(true);
        });
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        Ok(())
    })
}
