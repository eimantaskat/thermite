use serde::Deserialize;
use thermite::{Model, ToSchema};

#[derive(Deserialize, Model)]
struct Tag {
    label: String,
}

#[derive(Deserialize, Model)]
struct Item {
    #[model(default)]
    tag: Tag,
}

fn main() {
    let _ = Item::schema();
}
