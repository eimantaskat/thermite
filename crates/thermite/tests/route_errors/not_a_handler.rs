use thermite::get;

#[get("/")]
async fn count(count: i64) -> String {
    count.to_string()
}

fn main() {}
