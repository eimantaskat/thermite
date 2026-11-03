use thermite::SecurityScheme;

#[derive(SecurityScheme)]
#[security(http_basic)]
struct Admin(String);

fn main() {}
