use criterion::{Criterion, criterion_group};
use serde::{Deserialize, Serialize};
use thermite::http::header::CONTENT_TYPE;
use thermite::{App, Bytes, Json, Model, Path, Query, StatusCode};

use crate::support::{bench_send, request, runtime, start};

#[derive(Serialize, Model)]
struct Greeting {
    message: String,
}

#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: i64,
}

#[derive(Deserialize, Model)]
struct Search {
    q: Option<String>,
}

#[derive(Serialize, Model)]
struct Found {
    item_id: i64,
    q: Option<String>,
}

#[derive(Serialize, Deserialize, Model)]
struct Item {
    name: String,
    price: f64,
    quantity: i64,
    active: bool,
    tags: Vec<String>,
}

#[derive(Serialize, Deserialize, Model)]
struct Order {
    id: i64,
    items: Vec<Item>,
}

const LARGE_ORDER_ITEMS: i64 = 100;

async fn plaintext() -> &'static str {
    "Hello, World!"
}

async fn json() -> Json<Greeting> {
    Json(Greeting {
        message: "Hello, World!".to_owned(),
    })
}

async fn read_item(Path(path): Path<ItemPath>, Query(search): Query<Search>) -> Json<Found> {
    Json(Found {
        item_id: path.item_id,
        q: search.q,
    })
}

async fn create_item(Json(item): Json<Item>) -> Json<Item> {
    Json(item)
}

async fn create_order(Json(order): Json<Order>) -> Json<Order> {
    Json(order)
}

fn item(quantity: i64) -> Item {
    Item {
        name: format!("item {quantity}"),
        price: 9.5,
        quantity,
        active: true,
        tags: vec!["hot".to_owned(), "new".to_owned()],
    }
}

fn to_body(value: &impl Serialize) -> Bytes {
    Bytes::from(serde_json::to_vec(value).expect("a JSON body"))
}

fn endpoints(criterion: &mut Criterion) {
    let runtime = runtime();
    let app = App::new()
        .get("/", plaintext)
        .and_then(|app| app.get("/json", json))
        .and_then(|app| app.get("/items/{item_id}", read_item))
        .and_then(|app| app.post("/items", create_item))
        .and_then(|app| app.post("/orders", create_order))
        .expect("routes that register");
    let server = start(&runtime, app);
    let small = to_body(&item(1));
    let large = to_body(&Order {
        id: 1,
        items: (0..LARGE_ORDER_ITEMS).map(item).collect(),
    });

    let mut group = criterion.benchmark_group("endpoints");
    let mut bench = |name: &str, build: &dyn Fn() -> thermite::Request| {
        bench_send(&mut group, &runtime, name, &server, StatusCode::OK, build);
    };
    bench("plaintext", &|| {
        request(thermite::http::Request::get("/"), "")
    });
    bench("json", &|| {
        request(thermite::http::Request::get("/json"), "")
    });
    bench("path_and_query", &|| {
        request(thermite::http::Request::get("/items/42?q=hammer"), "")
    });
    bench("json_body/small", &|| {
        request(
            thermite::http::Request::post("/items").header(CONTENT_TYPE, "application/json"),
            small.clone(),
        )
    });
    bench("json_body/large", &|| {
        request(
            thermite::http::Request::post("/orders").header(CONTENT_TYPE, "application/json"),
            large.clone(),
        )
    });
    group.finish();
}

criterion_group!(benches, endpoints);
