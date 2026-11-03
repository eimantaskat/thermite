mod credentials;

use http::header::WWW_AUTHENTICATE;
use http::{HeaderName, HeaderValue, StatusCode};
use serde_json::{Value, json};

use crate::description::ResponseDescription;
use crate::http_error::HttpError;
use crate::schema::{JsonSchema, entries};

pub use credentials::{
    HttpAuthorizationCredentials, HttpBasicCredentials, InvalidCredentials, read_api_key,
    read_basic_credentials, read_bearer_credentials, read_bearer_token,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiKeyLocation {
    Query,
    Header,
    Cookie,
}

impl ApiKeyLocation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Header => "header",
            Self::Cookie => "cookie",
        }
    }
}

// what the docs show of a scheme, under a name that like FastAPI's defaults to the kind of scheme
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecuritySchemeDescription {
    scheme_name: String,
    description: Option<String>,
    kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    ApiKey(ApiKey),
    HttpBasic,
    HttpBearer,
    OAuth2Password { token_url: String },
}

#[derive(Debug, Clone)]
struct ApiKey {
    location: ApiKeyLocation,
    // as written, for the docs
    name: String,
    // parsed once for comparisons; None for other locations and for invalid names
    header_name: Option<HeaderName>,
}

impl ApiKey {
    // a parsed header name is lowercase, so two keys from one header in different cases are one
    fn declared_name(&self) -> &str {
        self.header_name
            .as_ref()
            .map_or(&self.name, HeaderName::as_str)
    }
}

impl PartialEq for ApiKey {
    fn eq(&self, other: &Self) -> bool {
        self.location == other.location && self.declared_name() == other.declared_name()
    }
}

impl Eq for ApiKey {}

impl SecuritySchemeDescription {
    fn new(scheme_name: &str, kind: Kind) -> Self {
        Self {
            scheme_name: scheme_name.to_owned(),
            description: None,
            kind,
        }
    }

    #[must_use]
    pub fn api_key(location: ApiKeyLocation, name: impl Into<String>) -> Self {
        let scheme_name = match location {
            ApiKeyLocation::Query => "APIKeyQuery",
            ApiKeyLocation::Header => "APIKeyHeader",
            ApiKeyLocation::Cookie => "APIKeyCookie",
        };
        let name = name.into();
        let header_name = match location {
            ApiKeyLocation::Header => HeaderName::from_bytes(name.as_bytes()).ok(),
            ApiKeyLocation::Query | ApiKeyLocation::Cookie => None,
        };
        let key = ApiKey {
            location,
            name,
            header_name,
        };
        Self::new(scheme_name, Kind::ApiKey(key))
    }

    #[must_use]
    pub fn http_basic() -> Self {
        Self::new("HTTPBasic", Kind::HttpBasic)
    }

    #[must_use]
    pub fn http_bearer() -> Self {
        Self::new("HTTPBearer", Kind::HttpBearer)
    }

    #[must_use]
    pub fn oauth2_password(token_url: impl Into<String>) -> Self {
        let token_url = token_url.into();
        Self::new("OAuth2PasswordBearer", Kind::OAuth2Password { token_url })
    }

    #[must_use]
    pub fn with_scheme_name(mut self, scheme_name: impl Into<String>) -> Self {
        self.scheme_name = scheme_name.into();
        self
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub(crate) fn scheme_name(&self) -> &str {
        &self.scheme_name
    }

    // the header an API key is read from, when it is not a valid header name
    pub(crate) fn invalid_header(&self) -> Option<&str> {
        match &self.kind {
            Kind::ApiKey(ApiKey {
                location: ApiKeyLocation::Header,
                name,
                header_name: None,
            }) => Some(name),
            _ => None,
        }
    }

    // what a front-end lists for a route that requires a scheme
    #[must_use]
    pub fn not_authenticated_response() -> ResponseDescription {
        ResponseDescription::new(StatusCode::UNAUTHORIZED, "Not authenticated")
            .with_schema(HttpError::text_schema())
    }

    // FastAPI's answer to a request without the credentials, whatever the scheme
    #[must_use]
    pub fn not_authenticated(&self) -> HttpError {
        let challenge = match self.kind {
            Kind::ApiKey(_) => "APIKey",
            Kind::HttpBasic => "Basic",
            Kind::HttpBearer | Kind::OAuth2Password { .. } => "Bearer",
        };
        HttpError::new(StatusCode::UNAUTHORIZED, "Not authenticated")
            .with_header(WWW_AUTHENTICATE, HeaderValue::from_static(challenge))
    }

    // in the key order of FastAPI's models
    pub(crate) fn json_schema(&self) -> JsonSchema {
        let kind = match self.kind {
            Kind::ApiKey(_) => "apiKey",
            Kind::HttpBasic | Kind::HttpBearer => "http",
            Kind::OAuth2Password { .. } => "oauth2",
        };
        let mut schema = entries([("type", kind.into())]);
        if let Some(description) = &self.description {
            schema.insert("description".to_owned(), description.as_str().into());
        }
        let mut insert = |key: &str, value: Value| schema.insert(key.to_owned(), value);
        match &self.kind {
            Kind::ApiKey(key) => {
                insert("in", key.location.as_str().into());
                insert("name", key.name.as_str().into());
            }
            Kind::HttpBasic => {
                insert("scheme", "basic".into());
            }
            Kind::HttpBearer => {
                insert("scheme", "bearer".into());
            }
            Kind::OAuth2Password { token_url } => {
                let flows = json!({ "password": { "scopes": {}, "tokenUrl": token_url } });
                insert("flows", flows);
            }
        }
        schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(description: &SecuritySchemeDescription) -> Value {
        Value::Object(description.json_schema())
    }

    #[test]
    fn documents_each_kind_of_scheme_as_fastapi_does() {
        assert_eq!(
            schema(&SecuritySchemeDescription::api_key(
                ApiKeyLocation::Cookie,
                "session"
            )),
            json!({ "type": "apiKey", "in": "cookie", "name": "session" })
        );
        assert_eq!(
            schema(&SecuritySchemeDescription::http_basic()),
            json!({ "type": "http", "scheme": "basic" })
        );
        assert_eq!(
            schema(&SecuritySchemeDescription::http_bearer()),
            json!({ "type": "http", "scheme": "bearer" })
        );
        assert_eq!(
            schema(&SecuritySchemeDescription::oauth2_password("token")),
            json!({
                "type": "oauth2",
                "flows": { "password": { "scopes": {}, "tokenUrl": "token" } }
            })
        );
    }

    #[test]
    fn puts_the_description_after_the_type() {
        let described = SecuritySchemeDescription::api_key(ApiKeyLocation::Query, "key")
            .with_description("A key from the dashboard");

        let keys: Vec<String> = described.json_schema().keys().cloned().collect();

        assert_eq!(keys, ["type", "description", "in", "name"]);
        assert_eq!(
            schema(&described)["description"],
            "A key from the dashboard"
        );
    }

    #[test]
    fn names_each_scheme_after_its_fastapi_class_unless_renamed() {
        let names = [
            SecuritySchemeDescription::api_key(ApiKeyLocation::Query, "key"),
            SecuritySchemeDescription::api_key(ApiKeyLocation::Header, "key"),
            SecuritySchemeDescription::api_key(ApiKeyLocation::Cookie, "key"),
            SecuritySchemeDescription::http_basic(),
            SecuritySchemeDescription::http_bearer(),
            SecuritySchemeDescription::oauth2_password("token"),
            SecuritySchemeDescription::http_basic().with_scheme_name("Admin"),
        ]
        .map(|description| description.scheme_name().to_owned());

        assert_eq!(
            names,
            [
                "APIKeyQuery",
                "APIKeyHeader",
                "APIKeyCookie",
                "HTTPBasic",
                "HTTPBearer",
                "OAuth2PasswordBearer",
                "Admin"
            ]
        );
    }

    #[test]
    fn challenges_with_the_scheme_fastapi_names() {
        let challenges = [
            SecuritySchemeDescription::api_key(ApiKeyLocation::Header, "x-key"),
            SecuritySchemeDescription::http_basic(),
            SecuritySchemeDescription::http_bearer(),
            SecuritySchemeDescription::oauth2_password("token"),
        ]
        .map(|description| description.not_authenticated());

        for (error, challenge) in challenges
            .iter()
            .zip(["APIKey", "Basic", "Bearer", "Bearer"])
        {
            assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(error.detail(), "Not authenticated");
            assert_eq!(error.headers()[WWW_AUTHENTICATE], challenge);
        }
    }

    #[test]
    fn describes_the_401_with_a_text_detail() {
        let response = SecuritySchemeDescription::not_authenticated_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.description(), "Not authenticated");
        assert_eq!(response.schema(), Some(&HttpError::text_schema()));
    }

    #[test]
    fn spots_an_api_key_header_that_is_not_a_header_name() {
        let invalid = SecuritySchemeDescription::api_key(ApiKeyLocation::Header, "x key");
        let query = SecuritySchemeDescription::api_key(ApiKeyLocation::Query, "x key");
        let valid = SecuritySchemeDescription::api_key(ApiKeyLocation::Header, "X-Key");

        assert_eq!(invalid.invalid_header(), Some("x key"));
        assert_eq!(query.invalid_header(), None);
        assert_eq!(valid.invalid_header(), None);
    }
}
