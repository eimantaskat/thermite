use thermite::Model;

#[derive(Model)]
struct Filter {
    #[model(default = 20)]
    limit: Option<i64>,
}

fn main() {}
