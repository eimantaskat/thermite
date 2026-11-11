use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use thermite::http::header::{
    ACCEPT_ENCODING, ACCEPT_RANGES, ALLOW, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE,
    CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, IF_RANGE, LAST_MODIFIED, LOCATION, RANGE,
};
use thermite::{
    App, GZipMiddleware, HttpError, RouteError, Router, StaticFiles, StatusCode, TestClient,
    TestResponse,
};

use crate::support::{RunningServer, TestResult};

const LARGE: usize = 200 * 1024;

// a directory of its own per test, removed when the test ends
struct Site {
    root: PathBuf,
}

impl Site {
    // `public` is what is served; `secret.txt` sits beside it, where no request may reach
    fn new() -> io::Result<Self> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "thermite-static-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&root);
        let site = Self { root };
        site.write("secret.txt", "secret")?;
        site.write("public/hello.txt", "Hello, world!")?;
        site.write("public/digits.txt", "0123456789")?;
        site.write("public/css/site.css", "body {}")?;
        site.write("public/a b.txt", "spaced")?;
        site.write("public/logo.png", "x".repeat(4000))?;
        site.write("public/large.txt", "x".repeat(LARGE))?;
        Ok(site)
    }

    fn with_html(self) -> io::Result<Self> {
        self.write("public/index.html", "<h1>Home</h1>")?;
        self.write("public/docs/index.html", "<h1>Docs</h1>")?;
        self.write("public/404.html", "<h1>Missing</h1>")?;
        Ok(self)
    }

    fn write(&self, path: &str, contents: impl AsRef<[u8]>) -> io::Result<()> {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }

    fn public(&self) -> PathBuf {
        self.root.join("public")
    }

    fn files(&self) -> StaticFiles {
        StaticFiles::new(self.public())
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

async fn client(files: StaticFiles) -> Result<TestClient, Box<dyn Error>> {
    let app = App::new().mount("/static", files)?;
    Ok(TestClient::start(app).await?)
}

fn header(response: &TestResponse, name: impl AsRef<str>) -> &str {
    response
        .headers()
        .get(name.as_ref())
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

#[tokio::test]
async fn serves_a_file_with_its_type_length_and_validators() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;

    let response = client.get("/static/hello.txt").send().await?;
    let nested = client.get("/static/css/site.css").send().await?;
    let spaced = client.get("/static/a%20b.txt").send().await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text()?, "Hello, world!");
    assert_eq!(header(&response, CONTENT_TYPE), "text/plain; charset=utf-8");
    assert_eq!(header(&response, CONTENT_LENGTH), "13");
    assert_eq!(header(&response, ACCEPT_RANGES), "bytes");
    let etag = header(&response, ETAG);
    assert!(etag.starts_with('"') && etag.ends_with("-d\""), "{etag}");
    assert!(header(&response, LAST_MODIFIED).ends_with(" GMT"));
    assert_eq!(nested.text()?, "body {}");
    assert_eq!(header(&nested, CONTENT_TYPE), "text/css; charset=utf-8");
    assert_eq!(spaced.text()?, "spaced");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_head_with_the_headers_of_get_and_no_body() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;

    let response = client.head("/static/hello.txt").send().await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, CONTENT_LENGTH), "13");
    assert_eq!(header(&response, CONTENT_TYPE), "text/plain; charset=utf-8");
    assert!(response.body().is_empty());
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn streams_a_large_file_in_chunks() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;

    let mut streamed = client.get("/static/large.txt").stream().await?;
    let mut chunks = Vec::new();
    while let Some(chunk) = streamed.chunk().await {
        chunks.push(chunk?);
    }

    assert_eq!(streamed.headers()[CONTENT_LENGTH], LARGE.to_string());
    assert!(chunks.len() > 1);
    assert!(chunks.iter().all(|chunk| chunk.len() <= 64 * 1024));
    assert_eq!(chunks.concat(), "x".repeat(LARGE).as_bytes());
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_404_for_what_is_missing_a_directory_and_a_path_climbing_out() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;

    for path in [
        "/static/missing.txt",
        "/static/css",
        "/static/",
        "/static",
        "/static/../secret.txt",
        "/static/css/../../secret.txt",
        "/static/%2E%2E/secret.txt",
        "/static/css/..%2F..%2Fsecret.txt",
        "/static/hello.txt%00",
    ] {
        let response = client.get(path).send().await?;

        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(response.json::<Value>()?, json!({"detail": "Not Found"}));
    }
    let inside = client.get("/static/css/../hello.txt").send().await?;
    assert_eq!(inside.text()?, "Hello, world!");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn following_symlinks_still_keeps_a_path_inside_the_directory() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files().follow_symlink(true)).await?;

    let file = client.get("/static/css/site.css").send().await?;
    let missing = client.get("/static/missing.txt").send().await?;
    let climbing = client.get("/static/../secret.txt").send().await?;

    assert_eq!(file.text()?, "body {}");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(climbing.status(), StatusCode::NOT_FOUND);
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn refuses_any_method_but_get_and_head() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;

    let response = client.post("/static/hello.txt").send().await?;

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header(&response, ALLOW), "GET, HEAD");
    assert_eq!(
        response.json::<Value>()?,
        json!({"detail": "Method Not Allowed"})
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_304_for_a_file_the_client_already_has() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;
    let first = client.get("/static/hello.txt").send().await?;
    let etag = header(&first, ETAG).to_owned();
    let last_modified = header(&first, LAST_MODIFIED).to_owned();

    for (name, value) in [
        (IF_NONE_MATCH, etag.clone()),
        (IF_NONE_MATCH, format!("\"other\", W/{etag}")),
        (IF_NONE_MATCH, " * ".to_owned()),
        (IF_MODIFIED_SINCE, last_modified.clone()),
        (
            IF_MODIFIED_SINCE,
            "Fri, 31 Dec 9999 23:59:59 GMT".to_owned(),
        ),
    ] {
        let response = client
            .get("/static/hello.txt")
            .header(name.clone(), value.as_str())
            .send()
            .await?;

        assert_eq!(
            response.status(),
            StatusCode::NOT_MODIFIED,
            "{name}: {value}"
        );
        assert_eq!(header(&response, ETAG), etag);
        assert!(!response.headers().contains_key(CONTENT_TYPE));
        assert!(!response.headers().contains_key(LAST_MODIFIED));
        assert!(response.body().is_empty());
    }
    let changed = client
        .get("/static/hello.txt")
        .header(IF_NONE_MATCH, "\"other\"")
        .header(IF_MODIFIED_SINCE, last_modified.as_str())
        .send()
        .await?;
    let older = client
        .get("/static/hello.txt")
        .header(IF_MODIFIED_SINCE, "Thu, 01 Jan 1970 00:00:00 GMT")
        .send()
        .await?;
    assert_eq!(changed.status(), StatusCode::OK);
    assert_eq!(older.status(), StatusCode::OK);
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_a_single_range_with_206() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;

    let middle = client
        .get("/static/digits.txt")
        .header(RANGE, "bytes=2-5")
        .send()
        .await?;
    let last = client
        .get("/static/digits.txt")
        .header(RANGE, "bytes=-3")
        .send()
        .await?;
    let head = client
        .head("/static/digits.txt")
        .header(RANGE, "bytes=2-5")
        .send()
        .await?;

    assert_eq!(middle.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(middle.text()?, "2345");
    assert_eq!(header(&middle, CONTENT_RANGE), "bytes 2-5/10");
    assert_eq!(header(&middle, CONTENT_LENGTH), "4");
    assert_eq!(header(&middle, CONTENT_TYPE), "text/plain; charset=utf-8");
    assert_eq!(last.text()?, "789");
    assert_eq!(header(&last, CONTENT_RANGE), "bytes 7-9/10");
    assert_eq!(head.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header(&head, CONTENT_LENGTH), "4");
    assert!(head.body().is_empty());
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_several_ranges_as_multipart_byteranges() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;

    let response = client
        .get("/static/digits.txt")
        .header(RANGE, "bytes=7-8, 0-1, 1-2")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    let content_type = header(&response, CONTENT_TYPE);
    let boundary = content_type
        .strip_prefix("multipart/byteranges; boundary=")
        .unwrap_or_default();
    assert_eq!(boundary.len(), 26, "{content_type}");
    let part = |range: &str, digits: &str| {
        format!(
            "--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Range: bytes {range}/10\r\n\r\n{digits}\r\n"
        )
    };
    let expected = format!("{}{}--{boundary}--", part("0-2", "012"), part("7-8", "78"));
    assert_eq!(response.text()?, expected);
    assert_eq!(
        header(&response, CONTENT_LENGTH),
        expected.len().to_string()
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_head_for_several_ranges_with_the_length_get_would_send() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;

    let head = client
        .head("/static/digits.txt")
        .header(RANGE, "bytes=0-1, 5-6")
        .send()
        .await?;
    let get = client
        .get("/static/digits.txt")
        .header(RANGE, "bytes=0-1, 5-6")
        .send()
        .await?;

    assert_eq!(head.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header(&head, CONTENT_LENGTH), get.body().len().to_string());
    assert!(head.body().is_empty());
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn refuses_a_range_it_cannot_read_or_satisfy() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;

    let malformed = client
        .get("/static/digits.txt")
        .header(RANGE, "lines=1-2")
        .send()
        .await?;
    let past_the_end = client
        .get("/static/digits.txt")
        .header(RANGE, "bytes=10-")
        .send()
        .await?;

    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    assert_eq!(malformed.text()?, "Only support bytes range");
    assert_eq!(
        header(&malformed, CONTENT_TYPE),
        "text/plain; charset=utf-8"
    );
    assert_eq!(past_the_end.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(header(&past_the_end, CONTENT_RANGE), "bytes */10");
    assert!(past_the_end.body().is_empty());
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn serves_a_range_only_while_if_range_matches() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files()).await?;
    let first = client.get("/static/digits.txt").send().await?;
    let etag = header(&first, ETAG).to_owned();
    let last_modified = header(&first, LAST_MODIFIED).to_owned();

    for (validator, status) in [
        (etag.clone(), StatusCode::PARTIAL_CONTENT),
        (last_modified, StatusCode::PARTIAL_CONTENT),
        (format!("W/{etag}"), StatusCode::OK),
        ("\"other\"".to_owned(), StatusCode::OK),
    ] {
        let response = client
            .get("/static/digits.txt")
            .header(RANGE, "bytes=0-1")
            .header(IF_RANGE, validator.as_str())
            .send()
            .await?;

        assert_eq!(response.status(), status, "{validator}");
    }
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn serves_index_html_and_404_html_in_html_mode() -> TestResult {
    let site = Site::new()?.with_html()?;
    let client = client(site.files().html(true)).await?;

    let home = client.get("/static/").send().await?;
    let docs = client.get("/static/docs/").send().await?;
    let missing = client
        .get("/static/missing.txt")
        .header(RANGE, "bytes=0-1")
        .send()
        .await?;
    let not_a_page = client.get("/static/css/").send().await?;

    assert_eq!(home.text()?, "<h1>Home</h1>");
    assert_eq!(header(&home, CONTENT_TYPE), "text/html; charset=utf-8");
    assert_eq!(docs.text()?, "<h1>Docs</h1>");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(missing.text()?, "<h1>Missing</h1>");
    assert!(missing.headers().contains_key(ETAG));
    assert_eq!(not_a_page.status(), StatusCode::NOT_FOUND);
    assert_eq!(not_a_page.text()?, "<h1>Missing</h1>");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn redirects_a_directory_to_its_path_with_a_trailing_slash() -> TestResult {
    let site = Site::new()?.with_html()?;
    let client = client(site.files().html(true)).await?;

    for (path, location) in [
        ("/static", "/static/"),
        ("/static/docs", "/static/docs/"),
        ("/static/docs?page=2", "/static/docs/?page=2"),
    ] {
        let response = client.get(path).send().await?;

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT, "{path}");
        assert_eq!(header(&response, LOCATION), location);
        assert!(response.body().is_empty());
    }
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn collapses_leading_slashes_so_a_redirect_never_names_another_host() -> TestResult {
    let site = Site::new()?.with_html()?;
    let client = TestClient::start(App::new().mount("/", site.files().html(true))?).await?;

    for (path, location) in [("//docs", "/docs/"), ("///docs?page=2", "/docs/?page=2")] {
        let response = client.get(path).send().await?;

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT, "{path}");
        assert_eq!(header(&response, LOCATION), location, "{path}");
    }
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn serves_the_404_page_for_a_path_climbing_out_in_html_mode() -> TestResult {
    let site = Site::new()?.with_html()?;
    let client = client(site.files().html(true)).await?;

    let response = client.get("/static/../secret.txt").send().await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.text()?, "<h1>Missing</h1>");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_json_404_in_html_mode_without_a_404_page() -> TestResult {
    let site = Site::new()?;
    let client = client(site.files().html(true)).await?;

    let response = client.get("/static/missing.txt").send().await?;

    assert_eq!(response.json::<Value>()?, json!({"detail": "Not Found"}));
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn routes_come_before_a_mount_at_the_root() -> TestResult {
    let site = Site::new()?.with_html()?;
    let app = App::new()
        .mount("/", site.files().html(true))?
        .get("/api/items", || async { "items" })?
        .post("/hello.txt", || async { "posted" })?;
    let client = TestClient::start(app).await?;

    let route = client.get("/api/items").send().await?;
    let posted = client.post("/hello.txt").send().await?;
    let file = client.get("/hello.txt").send().await?;
    let home = client.get("/").send().await?;
    let wrong_method = client.post("/api/items").send().await?;
    let docs = client.get("/docs").send().await?;

    assert_eq!(route.text()?, "items");
    assert_eq!(posted.text()?, "posted");
    assert_eq!(file.text()?, "Hello, world!");
    assert_eq!(home.text()?, "<h1>Home</h1>");
    // the mount matches every path, so it answers where the route's 405 would, as in Starlette
    assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header(&wrong_method, ALLOW), "GET, HEAD");
    assert_eq!(docs.status(), StatusCode::OK);
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn a_mount_inside_another_answers_its_own_paths() -> TestResult {
    let outer = Site::new()?;
    let inner = Site::new()?;
    inner.write("public/hello.txt", "Hello from inside")?;
    let app = App::new()
        .mount("/files", outer.files())?
        .mount("/files/inner", inner.files())?;
    let client = TestClient::start(app).await?;

    let outside = client.get("/files/hello.txt").send().await?;
    let inside = client.get("/files/inner/hello.txt").send().await?;

    assert_eq!(outside.text()?, "Hello, world!");
    assert_eq!(inside.text()?, "Hello from inside");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn a_router_mounts_under_its_prefix() -> TestResult {
    let site = Site::new()?;
    let router = Router::new()
        .prefix("/assets")
        .mount("/static", site.files());
    let app = App::new().include_router(Router::new().include_router(router))?;
    let client = TestClient::start(app).await?;

    let response = client.get("/assets/static/hello.txt").send().await?;

    assert_eq!(response.text()?, "Hello, world!");
    client.stop().await;
    Ok(())
}

#[test]
fn a_router_refuses_a_mount_path_without_a_slash_or_under_an_invalid_prefix() -> TestResult {
    let site = Site::new()?;
    let without_slash = Router::new().prefix("/api").mount("static", site.files());
    let bad_prefix = Router::new().prefix("/api/").mount("/static", site.files());
    let listed = format!("{without_slash:?}");

    let without_slash = App::new().include_router(without_slash);
    let bad_prefix = App::new().include_router(Router::new().include_router(bad_prefix));

    assert!(listed.contains("path: \"static\""), "{listed}");
    assert_eq!(
        without_slash.err(),
        Some(RouteError::InvalidMountPath {
            path: "static".to_owned()
        })
    );
    assert_eq!(
        bad_prefix.err(),
        Some(RouteError::InvalidPrefix {
            prefix: "/api/".to_owned(),
            path: "/static".to_owned()
        })
    );
    Ok(())
}

#[test]
fn refuses_a_mount_it_could_never_serve() -> TestResult {
    let site = Site::new()?;
    let missing = site.public().join("missing");

    let not_a_directory = App::new().mount("/static", StaticFiles::new(&missing));
    let unchecked = App::new().mount("/static", StaticFiles::new(&missing).check_dir(false));
    let twice = App::new()
        .mount("/static/", site.files())?
        .mount("/static", site.files());
    let relative = App::new().mount("static", site.files());

    let Err(error) = not_a_directory else {
        return Err("mounted a missing directory".into());
    };
    assert_eq!(
        error,
        RouteError::StaticDirectory {
            path: "/static".to_owned(),
            directory: missing.clone()
        }
    );
    assert!(error.to_string().ends_with("which is not a directory"));
    assert!(unchecked.is_ok());
    assert_eq!(
        twice.err(),
        Some(RouteError::MountConflict {
            path: "/static".to_owned()
        })
    );
    assert_eq!(
        relative.err(),
        Some(RouteError::InvalidMountPath {
            path: "static".to_owned()
        })
    );
    Ok(())
}

#[tokio::test]
async fn an_unchecked_directory_created_later_is_served() -> TestResult {
    let site = Site::new()?;
    let later = site.root.join("later");
    let client = client(StaticFiles::new(&later).check_dir(false)).await?;

    let before = client.get("/static/note.txt").send().await?;
    site.write("later/note.txt", "now here")?;
    let after = client.get("/static/note.txt").send().await?;

    assert_eq!(before.status(), StatusCode::NOT_FOUND);
    assert_eq!(after.text()?, "now here");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn its_errors_reach_the_exception_handlers() -> TestResult {
    let site = Site::new()?;
    let app = App::new()
        .exception_handler(|error: &HttpError| (StatusCode::GONE, error.to_string()))
        .mount("/static", site.files())?;
    let client = TestClient::start(app).await?;

    let response = client.get("/static/missing.txt").send().await?;

    assert_eq!(response.status(), StatusCode::GONE);
    assert_eq!(response.text()?, "404 Not Found: Not Found");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn gzip_compresses_a_whole_text_file_but_not_an_image_or_a_range() -> TestResult {
    let site = Site::new()?;
    let app = App::new()
        .middleware(GZipMiddleware::new())
        .mount("/static", site.files())?;
    let client = TestClient::start(app).await?;

    let text = client
        .get("/static/large.txt")
        .header(ACCEPT_ENCODING, "gzip")
        .send()
        .await?;
    let image = client
        .get("/static/logo.png")
        .header(ACCEPT_ENCODING, "gzip")
        .send()
        .await?;
    let range = client
        .get("/static/large.txt")
        .header(ACCEPT_ENCODING, "gzip")
        .header(RANGE, "bytes=0-999")
        .send()
        .await?;

    assert_eq!(header(&text, CONTENT_ENCODING), "gzip");
    assert!(text.body().len() < LARGE);
    assert_eq!(header(&image, CONTENT_TYPE), "image/png");
    assert!(!image.headers().contains_key(CONTENT_ENCODING));
    assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
    assert!(!range.headers().contains_key(CONTENT_ENCODING));
    assert_eq!(range.body().len(), 1000);
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn sends_files_and_ranges_over_the_wire() -> TestResult {
    let site = Site::new()?;
    let server = RunningServer::start(App::new().mount("/static", site.files())?).await?;
    let client = reqwest::Client::new();

    let large = client.get(server.url("/static/large.txt")).send().await?;
    let large_length = large.headers()[CONTENT_LENGTH].clone();
    let large_body = large.bytes().await?;
    let head = client.head(server.url("/static/large.txt")).send().await?;
    let range = client
        .get(server.url("/static/digits.txt"))
        .header(RANGE, "bytes=3-4")
        .send()
        .await?;
    let range_status = range.status();
    let range_body = range.text().await?;

    assert_eq!(large_length, LARGE.to_string());
    assert_eq!(large_body, "x".repeat(LARGE).as_bytes());
    assert_eq!(head.headers()[CONTENT_LENGTH], LARGE.to_string());
    assert_eq!(range_status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(range_body, "34");
    server.stop().await
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    #[tokio::test]
    async fn follows_a_symlink_out_of_the_directory_only_when_told_to() -> TestResult {
        let site = Site::new()?;
        symlink(site.root.join("secret.txt"), site.public().join("link.txt"))?;
        let strict = client(site.files()).await?;
        let following = client(site.files().follow_symlink(true)).await?;

        let refused = strict.get("/static/link.txt").send().await?;
        let followed = following.get("/static/link.txt").send().await?;

        assert_eq!(refused.status(), StatusCode::NOT_FOUND);
        assert_eq!(followed.text()?, "secret");
        strict.stop().await;
        following.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn answers_500_for_a_symlink_loop_as_starlette_does() -> TestResult {
        let site = Site::new()?;
        symlink(site.public().join("loop"), site.public().join("loop"))?;
        let client = client(site.files()).await?;

        let response = client.get("/static/loop").send().await?;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn answers_401_for_a_directory_it_may_not_read() -> TestResult {
        let site = Site::new()?;
        let locked = site.public().join("css");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;
        // root reads anything, so there is nothing to refuse
        let readable = fs::read_dir(&locked).is_ok();
        let strict = client(site.files()).await?;
        let following = client(site.files().follow_symlink(true)).await?;

        let refused = strict.get("/static/css/site.css").send().await?;
        let refused_following = following.get("/static/css/site.css").send().await?;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))?;

        if !readable {
            for response in [refused, refused_following] {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                assert_eq!(response.json::<Value>()?, json!({"detail": "Unauthorized"}));
            }
        }
        strict.stop().await;
        following.stop().await;
        Ok(())
    }
}
