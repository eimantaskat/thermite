#![deny(dead_code)]

use thermite::get;

#[get("/")]
async fn home() -> &'static str {
    "home"
}

fn main() {}
