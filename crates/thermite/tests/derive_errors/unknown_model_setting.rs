use thermite::Model;

#[derive(Model)]
struct Filter {
    #[model(optional)]
    limit: i64,
}

fn main() {}
