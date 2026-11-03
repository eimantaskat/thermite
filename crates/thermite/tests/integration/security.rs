// dependencies are written here the way users write them, as async fns that may not await
#![allow(clippy::unused_async_trait_impl)]

use std::convert::Infallible;

use serde_json::{Value, json};
use thermite::http::header::{AUTHORIZATION, COOKIE, WWW_AUTHENTICATE};
use thermite::{
    App, Dependency, Depends, HttpAuthorizationCredentials, HttpBasicCredentials, HttpError,
    SecurityScheme, StatusCode, TestClient,
};

use crate::support::TestResult;

#[derive(SecurityScheme)]
#[security(oauth2_password, token_url = "token")]
struct Token(String);

#[derive(SecurityScheme)]
#[security(api_key, header = "x-key", description = "The key of a service")]
struct ServiceKey(String);

#[derive(SecurityScheme)]
#[security(api_key, query = "api_key")]
struct QueryKey(String);

#[derive(SecurityScheme)]
#[security(api_key, cookie = "session")]
struct Session(String);

#[derive(SecurityScheme)]
#[security(http_basic, scheme_name = "Admin")]
struct Admin(HttpBasicCredentials);

#[derive(SecurityScheme)]
#[security(http_bearer)]
struct Bearer(HttpAuthorizationCredentials);

#[derive(Clone)]
struct CurrentUser(String);

impl Dependency for CurrentUser {
    type Needs = Token;
    type Rejection = Infallible;

    async fn resolve(Token(token): Token) -> Result<Self, Infallible> {
        Ok(Self(format!("user of {token}")))
    }
}

async fn read_me(Depends(CurrentUser(user)): Depends<CurrentUser>) -> String {
    user
}

async fn read_items(ServiceKey(key): ServiceKey) -> String {
    format!("service {key}")
}

async fn search(QueryKey(key): QueryKey) -> String {
    format!("query {key}")
}

async fn read_session(Session(session): Session) -> String {
    format!("session {session}")
}

async fn admin(Admin(credentials): Admin) -> String {
    format!("{}:{}", credentials.username, credentials.password)
}

async fn bearer(Bearer(credentials): Bearer) -> String {
    format!("{} {}", credentials.scheme, credentials.credentials)
}

async fn maybe_key(key: Option<ServiceKey>) -> String {
    key.map_or_else(|| "anonymous".to_owned(), |ServiceKey(key)| key)
}

async fn maybe_admin(admin: Option<Admin>) -> String {
    admin.map_or_else(
        || "anonymous".to_owned(),
        |Admin(credentials)| credentials.username,
    )
}

fn app() -> Result<App, thermite::RouteError> {
    App::new()
        .get("/users/me", read_me)?
        .get("/items", read_items)?
        .get("/search", search)?
        .get("/session", read_session)?
        .get("/admin", admin)?
        .get("/bearer", bearer)?
        .get("/maybe/key", maybe_key)?
        .get("/maybe/admin", maybe_admin)
}

async fn client() -> Result<TestClient, Box<dyn std::error::Error>> {
    Ok(TestClient::start(app()?).await?)
}

#[tokio::test]
async fn hands_each_scheme_its_credentials() -> TestResult {
    let client = client().await?;

    let me = client
        .get("/users/me")
        .header(AUTHORIZATION, "Bearer abc")
        .send()
        .await?;
    let items = client.get("/items").header("x-key", "k1").send().await?;
    let search = client.get("/search?api_key=k2").send().await?;
    let session = client
        .get("/session")
        .header(COOKIE, "session=s1")
        .send()
        .await?;
    // "alice:secret"
    let admin = client
        .get("/admin")
        .header(AUTHORIZATION, "Basic YWxpY2U6c2VjcmV0")
        .send()
        .await?;
    let bearer = client
        .get("/bearer")
        .header(AUTHORIZATION, "bearer xyz")
        .send()
        .await?;
    client.stop().await;

    assert_eq!(me.text()?, "user of abc");
    assert_eq!(items.text()?, "service k1");
    assert_eq!(search.text()?, "query k2");
    assert_eq!(session.text()?, "session s1");
    assert_eq!(admin.text()?, "alice:secret");
    assert_eq!(bearer.text()?, "bearer xyz");
    Ok(())
}

#[tokio::test]
async fn answers_401_with_the_challenge_of_the_scheme_like_fastapi() -> TestResult {
    let client = client().await?;
    let mut answers = Vec::new();

    for (path, authorization) in [
        ("/users/me", "Basic YWxpY2U6c2VjcmV0"),
        ("/items", ""),
        ("/search", ""),
        ("/session", ""),
        ("/admin", "Bearer abc"),
        ("/bearer", "Bearer "),
        ("/bearer", "Basic YWxpY2U6c2VjcmV0"),
    ] {
        let mut request = client.get(path);
        if !authorization.is_empty() {
            request = request.header(AUTHORIZATION, authorization);
        }
        let response = request.send().await?;
        let challenge = response.headers()[WWW_AUTHENTICATE].to_str()?.to_owned();
        answers.push((response.status(), challenge, response.json::<Value>()?));
    }
    client.stop().await;

    let not_authenticated = json!({ "detail": "Not authenticated" });
    let expected = [
        "Bearer", "APIKey", "APIKey", "APIKey", "Basic", "Bearer", "Bearer",
    ]
    .map(|challenge| {
        (
            StatusCode::UNAUTHORIZED,
            challenge.to_owned(),
            not_authenticated.clone(),
        )
    });
    assert_eq!(answers, expected);
    Ok(())
}

#[tokio::test]
async fn optional_credentials_are_none_when_missing_but_bad_basic_ones_are_refused() -> TestResult {
    let client = client().await?;

    let missing_key = client.get("/maybe/key").send().await?;
    let key = client
        .get("/maybe/key")
        .header("x-key", "k1")
        .send()
        .await?;
    let missing_admin = client
        .get("/maybe/admin")
        .header(AUTHORIZATION, "Bearer abc")
        .send()
        .await?;
    let bad_admin = client
        .get("/maybe/admin")
        .header(AUTHORIZATION, "Basic not-base64")
        .send()
        .await?;
    client.stop().await;

    assert_eq!(missing_key.text()?, "anonymous");
    assert_eq!(key.text()?, "k1");
    assert_eq!(missing_admin.text()?, "anonymous");
    assert_eq!(bad_admin.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(bad_admin.headers()[WWW_AUTHENTICATE], "Basic");
    assert_eq!(
        bad_admin.json::<Value>()?,
        json!({ "detail": "Not authenticated" })
    );
    Ok(())
}

#[tokio::test]
async fn the_401_reaches_an_exception_handler_for_http_errors() -> TestResult {
    let app = app()?.exception_handler(|error: &HttpError| {
        let challenge = error.headers()[WWW_AUTHENTICATE]
            .to_str()
            .unwrap_or_default();
        (
            StatusCode::FORBIDDEN,
            format!("{}: {challenge}", error.status()),
        )
    });
    let client = TestClient::start(app).await?;

    let response = client.get("/items").send().await?;
    client.stop().await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(response.text()?, "401 Unauthorized: APIKey");
    Ok(())
}

#[tokio::test]
async fn documents_the_schemes_and_the_401_of_required_credentials() -> TestResult {
    let client = client().await?;

    let document: Value = client.get("/openapi.json").send().await?.json()?;
    client.stop().await;

    assert_eq!(
        document["components"]["securitySchemes"],
        json!({
            "OAuth2PasswordBearer": {
                "type": "oauth2",
                "flows": { "password": { "scopes": {}, "tokenUrl": "token" } }
            },
            "APIKeyHeader": {
                "type": "apiKey",
                "description": "The key of a service",
                "in": "header",
                "name": "x-key"
            },
            "APIKeyQuery": { "type": "apiKey", "in": "query", "name": "api_key" },
            "APIKeyCookie": { "type": "apiKey", "in": "cookie", "name": "session" },
            "Admin": { "type": "http", "scheme": "basic" },
            "HTTPBearer": { "type": "http", "scheme": "bearer" }
        })
    );
    let me = &document["paths"]["/users/me"]["get"];
    assert_eq!(me["security"], json!([{ "OAuth2PasswordBearer": [] }]));
    assert_eq!(
        me["responses"]["401"],
        json!({
            "description": "Not authenticated",
            "content": {
                "application/json": {
                    "schema": {
                        "properties": { "detail": { "title": "Detail", "type": "string" } },
                        "required": ["detail"],
                        "type": "object"
                    }
                }
            }
        })
    );
    let maybe = &document["paths"]["/maybe/key"]["get"];
    assert_eq!(maybe["security"], json!([{ "APIKeyHeader": [] }]));
    assert!(maybe["responses"].get("401").is_none());
    Ok(())
}
