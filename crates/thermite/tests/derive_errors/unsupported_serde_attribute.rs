use serde::Deserialize;
use thermite::Model;

#[derive(Deserialize, Model)]
struct Item {
    name: String,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Deserialize)]
struct Extra {
    color: String,
}

fn main() {}
