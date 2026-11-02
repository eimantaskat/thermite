use thermite::get;

struct Items;

impl Items {
    #[get("/items")]
    async fn list(&self) -> &'static str {
        "items"
    }
}

fn main() {}
