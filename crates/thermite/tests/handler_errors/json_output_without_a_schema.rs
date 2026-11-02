use serde::Serialize;
use thermite::{App, Json};

#[derive(Serialize)]
struct Item {
    name: String,
}

async fn read_item() -> Json<Item> {
    Json(Item {
        name: "anvil".to_owned(),
    })
}

fn main() {
    let _ = App::new().get("/items", read_item);
}
