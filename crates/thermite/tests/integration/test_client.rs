// resources are written here the way users write them, as async fns that may not await
#![allow(clippy::unused_async_trait_impl)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use thermite::http::header::{CONTENT_LENGTH, CONTENT_TYPE, HOST};
use thermite::{
    App, Arguments, FromRequest, Header, Json, Method, Model, Path, Query, Request, Resource,
    ServerError, StartupError, State, StatusCode, TestClient, TestRequestError,
};

use crate::support::TestResult;

#[derive(Serialize, Deserialize, Model)]
struct Item {
    name: String,
    price: f64,
}

#[derive(Deserialize, Model)]
struct ItemPath {
    item_id: i64,
}

#[derive(Deserialize, Model)]
struct Paging {
    limit: i64,
}

#[derive(Deserialize, Model)]
struct Client {
    x_token: String,
}

#[derive(Serialize, Deserialize, Model)]
struct Stored {
    id: i64,
    limit: i64,
    token: String,
    item: Item,
}

fn items() -> Result<App, Box<dyn Error>> {
    Ok(App::new().put(
        "/items/{item_id}",
        |Path(path): Path<ItemPath>,
         Query(paging): Query<Paging>,
         Header(client): Header<Client>,
         Json(item): Json<Item>| async move {
            Json(Stored {
                id: path.item_id,
                limit: paging.limit,
                token: client.x_token,
                item,
            })
        },
    )?)
}

#[tokio::test]
async fn sends_the_path_query_headers_and_json_body_to_the_route() -> TestResult {
    let client = TestClient::start(items()?).await?;

    let response = client
        .put("/items/7?limit=3")
        .header("x-token", "secret")
        .json(&Item {
            name: "hammer".to_owned(),
            price: 9.5,
        })
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/json");
    let stored: Stored = response.json()?;
    assert_eq!(
        (stored.id, stored.limit, stored.token.as_str()),
        (7, 3, "secret")
    );
    assert_eq!(
        (stored.item.name.as_str(), stored.item.price),
        ("hammer", 9.5)
    );
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn answers_validation_errors_and_unknown_routes_as_the_server_does() -> TestResult {
    let client = TestClient::start(items()?).await?;

    let invalid = client
        .put("/items/seven?limit=3")
        .header("x-token", "secret")
        .body(r#"{"name":"hammer","price":9.5}"#)
        .header("content-type", "application/json")
        .send()
        .await?;
    let missing = client.get("/missing").send().await?;
    let wrong_method = client.request(Method::POST, "/items/7").send().await?;

    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        invalid.text()?,
        r#"{"detail":[{"type":"int_parsing","loc":["path","item_id"],"msg":"Input should be a valid integer, unable to parse string as an integer","input":"seven"}]}"#
    );
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn serves_the_docs_built_when_the_app_starts() -> TestResult {
    let client = TestClient::start(items()?).await?;

    let document: serde_json::Value = client.get("/openapi.json").send().await?.json()?;
    let page = client.head("/docs").send().await?;

    assert!(document["paths"]["/items/{item_id}"]["put"].is_object());
    assert_eq!(page.status(), StatusCode::OK);
    assert!(page.body().is_empty());
    assert_ne!(page.headers()["content-length"], "0");
    client.stop().await;
    Ok(())
}

// every value of the framing headers, which no extractor reads
struct Framing(String);

impl FromRequest for Framing {
    type Rejection = Infallible;

    fn from_request(
        request: &Request,
        _: &Arguments,
    ) -> impl Future<Output = Result<Self, Infallible>> + Send {
        let values = |name| {
            let values: Vec<_> = request
                .headers()
                .get_all(name)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .collect();
            if values.is_empty() {
                "none".to_owned()
            } else {
                values.join(", ")
            }
        };
        ready(Ok(Self(format!(
            "{} | {} | {}",
            values(HOST),
            values(CONTENT_LENGTH),
            values(CONTENT_TYPE)
        ))))
    }
}

fn echoing_headers() -> Result<App, Box<dyn Error>> {
    let mut app = App::new();
    for method in [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ] {
        app = app.route(
            method,
            "/",
            |Framing(framing): Framing| async move { framing },
        )?;
    }
    Ok(app)
}

#[tokio::test]
async fn sends_the_host_and_length_a_client_would() -> TestResult {
    let client = TestClient::start(echoing_headers()?).await?;

    let with_body = client.post("/").body("abc").send().await?;
    let mut empty = Vec::new();
    for request in [
        client.post("/"),
        client.put("/"),
        client.patch("/"),
        client.get("/"),
        client.delete("/"),
    ] {
        empty.push(request.send().await?.text()?.to_owned());
    }
    let own_host = client
        .post("/")
        .header("host", "example.com")
        .send()
        .await?;

    assert_eq!(with_body.text()?, "testserver | 3 | none");
    assert_eq!(
        empty,
        [
            "testserver | 0 | none",
            "testserver | 0 | none",
            "testserver | 0 | none",
            "testserver | none | none",
            "testserver | none | none"
        ]
    );
    assert_eq!(own_host.text()?, "example.com | 0 | none");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn a_json_body_sets_one_content_type_unless_the_test_set_its_own() -> TestResult {
    let client = TestClient::start(echoing_headers()?).await?;

    let own_type = client
        .post("/")
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .json(&1)
        .send()
        .await?;
    let twice = client.post("/").json(&1).json(&22).send().await?;
    let replaced = client.post("/").json(&1).body("raw").send().await?;

    assert_eq!(
        own_type.text()?,
        "testserver | 1 | application/json; charset=utf-8"
    );
    assert_eq!(twice.text()?, "testserver | 2 | application/json");
    assert_eq!(replaced.text()?, "testserver | 3 | none");
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn refuses_a_body_over_the_limit_with_the_server_413() -> TestResult {
    let client = TestClient::start(echoing_headers()?)
        .await?
        .with_body_limit(4);

    let fits = client.post("/").body("abcd").send().await?;
    let too_large = client.post("/").body("abcde").send().await?;

    assert_eq!(fits.status(), StatusCode::OK);
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(too_large.text()?, r#"{"detail":"Content Too Large"}"#);
    client.stop().await;
    Ok(())
}

// a handler that panics is what these tests are about
#[allow(clippy::panic)]
async fn fails() {
    panic!("handler failed");
}

fn panicking() -> Result<App, Box<dyn Error>> {
    Ok(App::new().get("/", fails)?)
}

#[tokio::test]
async fn a_panicking_handler_fails_the_test_with_its_own_message() -> TestResult {
    let client = Arc::new(TestClient::start(panicking()?).await?);

    let outcome = tokio::spawn({
        let client = Arc::clone(&client);
        async move {
            client
                .get("/")
                .send()
                .await
                .map(|response| response.status())
        }
    })
    .await;

    let Err(error) = outcome else {
        return Err("the panic did not reach the test".into());
    };
    assert_eq!(
        error.into_panic().downcast_ref::<&str>(),
        Some(&"handler failed")
    );
    Ok(())
}

#[tokio::test]
async fn catch_panics_answers_a_panic_with_the_server_500() -> TestResult {
    let client = TestClient::start(panicking()?).await?.catch_panics();

    let response = client.get("/").send().await?;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.text()?, "Internal Server Error");
    client.stop().await;
    Ok(())
}

#[derive(Default)]
struct Log(Mutex<Vec<&'static str>>);

impl Log {
    fn record(&self, event: &'static str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn recorded(&self) -> Vec<&'static str> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct Database {
    log: Arc<Log>,
}

impl Resource for Database {
    type Needs = State<Log>;
    type Error = Infallible;

    async fn start(State(log): State<Log>) -> Result<Self, Infallible> {
        log.record("start");
        Ok(Self { log })
    }

    async fn stop(&self) {
        self.log.record("stop");
    }
}

#[tokio::test]
async fn starts_the_app_resources_before_the_first_request_and_stops_them_on_stop() -> TestResult {
    let app = App::new()
        .state(Log::default())
        .get("/", |State(database): State<Database>| async move {
            database.log.record("request");
            database.log.recorded().join(", ")
        })?
        .resource::<Database>();
    let client = TestClient::start(app).await?;

    let response = client.get("/").send().await?;
    client.stop().await;

    assert_eq!(response.text()?, "start, request");
    Ok(())
}

#[tokio::test]
async fn reports_an_app_that_fails_to_start() -> TestResult {
    let app = App::new().get("/", |_: State<Log>| async {})?;

    let outcome = TestClient::start(app).await;

    let Some(ServerError::Startup(error)) = outcome.err() else {
        return Err("the app started".into());
    };
    assert!(matches!(
        error.downcast_ref::<StartupError>(),
        Some(StartupError::MissingState { .. })
    ));
    Ok(())
}

#[tokio::test]
async fn reports_a_request_it_cannot_build_when_sent() -> TestResult {
    let client = TestClient::start(App::new()).await?;

    let relative = client.get("items").send().await.err();
    let header = client.get("/").header("bad header", "x").send().await.err();
    let body = client
        .post("/")
        // JSON object keys are strings
        .json(&BTreeMap::from([((1, 2), 3)]))
        .send()
        .await
        .err();

    assert!(matches!(
        relative,
        Some(TestRequestError::MissingLeadingSlash { path }) if path == "items"
    ));
    assert!(matches!(header, Some(TestRequestError::Http(_))));
    assert!(matches!(body, Some(TestRequestError::Body(_))));
    client.stop().await;
    Ok(())
}

#[tokio::test]
async fn each_method_shortcut_sends_its_method() -> TestResult {
    let methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::HEAD,
        Method::OPTIONS,
        Method::TRACE,
    ];
    let mut app = App::new();
    for method in methods.clone() {
        let shown = method.to_string();
        app = app.route(method, "/", move || {
            let shown = shown.clone();
            async move { shown }
        })?;
    }
    let client = TestClient::start(app).await?;

    let requests = [
        client.get("/"),
        client.post("/"),
        client.put("/"),
        client.patch("/"),
        client.delete("/"),
        client.head("/"),
        client.options("/"),
        client.trace("/"),
    ];
    let mut answered = Vec::new();
    for request in requests {
        let response = request.send().await?.into_response();
        let length = response.headers()["content-length"].to_str()?.to_owned();
        answered.push((response.body().clone(), length));
    }
    client.stop().await;

    for ((body, length), method) in answered.into_iter().zip(methods) {
        let expected_body = if method == Method::HEAD {
            ""
        } else {
            method.as_str()
        };
        assert_eq!(
            (body.as_ref(), length),
            (expected_body.as_bytes(), method.as_str().len().to_string())
        );
    }
    Ok(())
}
