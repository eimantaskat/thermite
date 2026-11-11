// written as async fns, as a dependency that reads a database would be
#![allow(clippy::unused_async_trait_impl)]

// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::http::HeaderValue;
use thermite::http::header::WWW_AUTHENTICATE;
use thermite::{
    App, Dependency, Depends, Form, HttpError, Json, Model, RouteError, SecurityScheme, Server,
    StatusCode, get, post, routes,
};
// ANCHOR_END: imports

// ANCHOR: scheme
#[derive(SecurityScheme)]
#[security(oauth2_password, token_url = "token")]
struct Token(String);
// ANCHOR_END: scheme

// ANCHOR: users
struct StoredUser {
    username: &'static str,
    full_name: &'static str,
    password: &'static str,
    disabled: bool,
}

static USERS: [StoredUser; 2] = [
    StoredUser {
        username: "johndoe",
        full_name: "John Doe",
        password: "secret",
        disabled: false,
    },
    StoredUser {
        username: "alice",
        full_name: "Alice Wonderson",
        password: "secret2",
        disabled: true,
    },
];

fn find_user(username: &str) -> Option<&'static StoredUser> {
    USERS.iter().find(|user| user.username == username)
}
// ANCHOR_END: users

// ANCHOR: login
#[derive(Deserialize, Model)]
struct Login {
    username: String,
    password: String,
}

#[derive(Serialize, Model)]
struct AccessToken {
    access_token: String,
    token_type: String,
}

#[post("/token")]
async fn login(Form(login): Form<Login>) -> Result<Json<AccessToken>, HttpError> {
    match find_user(&login.username) {
        Some(user) if user.password == login.password => Ok(Json(AccessToken {
            access_token: user.username.to_owned(),
            token_type: "bearer".to_owned(),
        })),
        _ => Err(HttpError::new(
            StatusCode::BAD_REQUEST,
            "Incorrect username or password",
        )),
    }
}
// ANCHOR_END: login

// ANCHOR: current_user
#[derive(Clone, Serialize, Model)]
struct User {
    username: String,
    full_name: String,
}

#[derive(Clone)]
struct CurrentUser {
    user: User,
    disabled: bool,
}

impl Dependency for CurrentUser {
    type Needs = Token;
    type Rejection = HttpError;

    async fn resolve(Token(token): Token) -> Result<Self, HttpError> {
        let Some(stored) = find_user(&token) else {
            return Err(HttpError::new(
                StatusCode::UNAUTHORIZED,
                "Invalid authentication credentials",
            )
            .with_header(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer")));
        };
        Ok(Self {
            user: User {
                username: stored.username.to_owned(),
                full_name: stored.full_name.to_owned(),
            },
            disabled: stored.disabled,
        })
    }
}

#[derive(Clone)]
struct ActiveUser(User);

impl Dependency for ActiveUser {
    type Needs = Depends<CurrentUser>;
    type Rejection = HttpError;

    async fn resolve(Depends(current): Depends<CurrentUser>) -> Result<Self, HttpError> {
        if current.disabled {
            return Err(HttpError::new(StatusCode::BAD_REQUEST, "Inactive user"));
        }
        Ok(Self(current.user))
    }
}

#[get("/users/me")]
async fn read_me(Depends(ActiveUser(user)): Depends<ActiveUser>) -> Json<User> {
    Json(user)
}
// ANCHOR_END: current_user

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![login, read_me])
}
// ANCHOR_END: app

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
    use thermite::http::header::AUTHORIZATION;

    use super::*;

    #[tokio::test]
    async fn logs_in_and_reads_the_user_with_the_token() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let token = client
            .post("/token")
            .form([("username", "johndoe"), ("password", "secret")])
            .send()
            .await?;
        let me = client
            .get("/users/me")
            .header(AUTHORIZATION, "Bearer johndoe")
            .send()
            .await?;

        assert_eq!(
            token.text()?,
            r#"{"access_token":"johndoe","token_type":"bearer"}"#
        );
        assert_eq!(
            me.text()?,
            r#"{"username":"johndoe","full_name":"John Doe"}"#
        );
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers and parts of the schema as they are
#[cfg(test)]
mod shown_tests {
    use serde_json::json;
    use thermite::http::header::AUTHORIZATION;
    use thermite::{TestClient, Value};

    use super::*;

    #[tokio::test]
    async fn refuses_a_wrong_password() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/token")
            .form([("username", "johndoe"), ("password", "guess")])
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.text()?,
            r#"{"detail":"Incorrect username or password"}"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn answers_401_without_a_token_or_with_an_unknown_one() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let missing = client.get("/users/me").send().await?;
        let unknown = client
            .get("/users/me")
            .header(AUTHORIZATION, "Bearer nobody")
            .send()
            .await?;

        for response in [&missing, &unknown] {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()[WWW_AUTHENTICATE], "Bearer");
        }
        assert_eq!(missing.text()?, r#"{"detail":"Not authenticated"}"#);
        assert_eq!(
            unknown.text()?,
            r#"{"detail":"Invalid authentication credentials"}"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn refuses_a_disabled_user() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .get("/users/me")
            .header(AUTHORIZATION, "Bearer alice")
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.text()?, r#"{"detail":"Inactive user"}"#);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn the_docs_list_the_scheme_and_the_routes_needing_it() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let openapi: Value = client.get("/openapi.json").send().await?.json()?;

        assert_eq!(
            openapi["components"]["securitySchemes"],
            json!({
              "OAuth2PasswordBearer": {
                "type": "oauth2",
                "flows": {"password": {"scopes": {}, "tokenUrl": "token"}}
              }
            })
        );
        assert_eq!(
            openapi["paths"]["/users/me"]["get"]["security"],
            json!([{"OAuth2PasswordBearer": []}])
        );
        client.stop().await;
        Ok(())
    }
}
