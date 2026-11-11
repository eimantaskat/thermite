// ANCHOR: imports
use std::error::Error;

use thermite::{App, RouteError, Server, StaticFiles, get, routes};
// ANCHOR_END: imports

// ANCHOR: app
#[get("/")]
async fn root() -> &'static str {
    "Hello World"
}

fn app() -> Result<App, RouteError> {
    App::new()
        .routes(routes![root])?
        .mount("/static", StaticFiles::new("static"))
}
// ANCHOR_END: app

// shown on the page and used only by its test
#[cfg(test)]
// ANCHOR: single_page
fn single_page_app() -> Result<App, RouteError> {
    App::new()
        .routes(routes![root])?
        .mount("/", StaticFiles::new("static").html(true))
}
// ANCHOR_END: single_page

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = Server::bind("127.0.0.1:8000").await?;
    server.serve(app()?).await?;
    Ok(())
}

// ANCHOR: test
#[cfg(test)]
mod tests {
    use thermite::TestClient;

    use super::*;

    #[tokio::test]
    async fn serves_the_files_of_the_directory() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/static/hello.txt").send().await?;

        assert_eq!(response.text()?, "Hello from a file");
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers as they are
#[cfg(test)]
mod shown_tests {
    use thermite::http::header::{CONTENT_TYPE, ETAG, IF_NONE_MATCH, LAST_MODIFIED, RANGE};
    use thermite::{StatusCode, TestClient};

    use super::*;

    #[tokio::test]
    async fn guesses_the_type_and_answers_404_for_a_missing_file() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let found = client.get("/static/hello.txt").send().await?;
        let missing = client.get("/static/missing.txt").send().await?;
        let root = client.get("/").send().await?;

        assert_eq!(found.headers()[CONTENT_TYPE], "text/plain; charset=utf-8");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.text()?, r#"{"detail":"Not Found"}"#);
        assert_eq!(root.text()?, "Hello World");
        client.stop().await;
        Ok(())
    }

    #[test]
    fn a_missing_directory_is_refused_when_mounted() {
        let error = App::new()
            .mount("/static", StaticFiles::new("no-such-directory"))
            .err()
            .map(|error| error.to_string());

        assert_eq!(
            error.as_deref(),
            Some(concat!(
                "the static files mounted at `/static` come from `no-such-directory`, ",
                "which is not a directory"
            ))
        );
    }

    #[tokio::test]
    async fn html_mode_serves_the_index_of_a_directory() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(single_page_app()?).await?;

        let index = client.get("/index.html").send().await?;
        let root = client.get("/").send().await?;
        let file = client.get("/hello.txt").send().await?;

        assert_eq!(index.headers()[CONTENT_TYPE], "text/html; charset=utf-8");
        assert!(index.text()?.contains("Served from docs_src/static."));
        assert_eq!(root.text()?, "Hello World");
        assert_eq!(file.text()?, "Hello from a file");
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn answers_with_validators_and_ranges() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let whole = client.get("/static/hello.txt").send().await?;
        let part = client
            .get("/static/hello.txt")
            .header(RANGE, "bytes=0-4")
            .send()
            .await?;

        assert!(whole.headers().contains_key(ETAG));
        assert!(whole.headers().contains_key(LAST_MODIFIED));
        let etag = whole.headers()[ETAG].clone();
        let cached = client
            .get("/static/hello.txt")
            .header(IF_NONE_MATCH, etag)
            .send()
            .await?;
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(part.text()?, "Hello");
        client.stop().await;
        Ok(())
    }
}
