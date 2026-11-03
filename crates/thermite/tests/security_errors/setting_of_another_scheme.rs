use thermite::SecurityScheme;

#[derive(SecurityScheme)]
#[security(http_bearer, token_url = "token")]
struct Token(String);

fn main() {}
