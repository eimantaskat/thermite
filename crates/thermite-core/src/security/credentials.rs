use std::borrow::Cow;
use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use http::header::{AUTHORIZATION, COOKIE};

use super::ApiKeyLocation;
use crate::handler::Request;
use crate::parse::{decode_latin1, parse_cookies, parse_query, strip};

#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HttpBasicCredentials {
    pub username: String,
    pub password: String,
}

// the scheme as sent, such as `bearer`, and what follows it
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HttpAuthorizationCredentials {
    pub scheme: String,
    pub credentials: String,
}

// kept out of logs, unlike in FastAPI's models
impl fmt::Debug for HttpBasicCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpBasicCredentials")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for HttpAuthorizationCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpAuthorizationCredentials")
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the basic credentials are not `username:password` in ASCII, encoded as base64")]
pub struct InvalidCredentials;

// like FastAPI's API key schemes, an empty value is no key
#[must_use]
pub fn read_api_key(request: &Request, location: ApiKeyLocation, name: &str) -> Option<String> {
    let key = match location {
        // like Starlette, the last value of a repeated name counts
        ApiKeyLocation::Query => parse_query(request.uri().query().unwrap_or(""))
            .into_iter()
            .rev()
            .find(|(found, _)| found == name)
            .map(|(_, value)| value.into_owned()),
        ApiKeyLocation::Header => request
            .headers()
            .get(name)
            .map(|value| decode_latin1(value.as_bytes()).into_owned()),
        ApiKeyLocation::Cookie => request.headers().get(COOKIE).and_then(|header| {
            parse_cookies(&decode_latin1(header.as_bytes()))
                .into_iter()
                .rev()
                .find(|(found, _)| *found == name)
                .map(|(_, value)| value.into_owned())
        }),
    };
    key.filter(|key| !key.is_empty())
}

// OAuth2PasswordBearer's token, which may be empty
#[must_use]
pub fn read_bearer_token(request: &Request) -> Option<String> {
    let authorization = authorization(request)?;
    let (scheme, token) = split_scheme(&authorization);
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.to_owned())
}

// HTTPBearer's, which needs both parts
#[must_use]
pub fn read_bearer_credentials(request: &Request) -> Option<HttpAuthorizationCredentials> {
    let authorization = authorization(request)?;
    let (scheme, credentials) = split_scheme(&authorization);
    (scheme.eq_ignore_ascii_case("bearer") && !credentials.is_empty()).then(|| {
        HttpAuthorizationCredentials {
            scheme: scheme.to_owned(),
            credentials: credentials.to_owned(),
        }
    })
}

// none without basic credentials, and an error when they do not decode, which FastAPI refuses even when they are optional
pub fn read_basic_credentials(
    request: &Request,
) -> Result<Option<HttpBasicCredentials>, InvalidCredentials> {
    let Some(authorization) = authorization(request) else {
        return Ok(None);
    };
    let (scheme, encoded) = split_scheme(&authorization);
    if !scheme.eq_ignore_ascii_case("basic") {
        return Ok(None);
    }
    let decoded = STANDARD.decode(encoded).map_err(|_| InvalidCredentials)?;
    let text = String::from_utf8(decoded)
        .ok()
        .filter(|text| text.is_ascii())
        .ok_or(InvalidCredentials)?;
    let (username, password) = text.split_once(':').ok_or(InvalidCredentials)?;
    Ok(Some(HttpBasicCredentials {
        username: username.to_owned(),
        password: password.to_owned(),
    }))
}

// like Starlette, the first Authorization header, and an empty one is none
fn authorization(request: &Request) -> Option<Cow<'_, str>> {
    request
        .headers()
        .get(AUTHORIZATION)
        .map(|value| decode_latin1(value.as_bytes()))
        .filter(|authorization| !authorization.is_empty())
}

// FastAPI's get_authorization_scheme_param: split at the first space, and strip what follows
fn split_scheme(authorization: &str) -> (&str, &str) {
    let (scheme, parameter) = authorization.split_once(' ').unwrap_or((authorization, ""));
    (scheme, strip(parameter))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use bytes::Bytes;
    use http::HeaderValue;

    use super::*;

    fn request(uri: &str, headers: &[(&str, &[u8])]) -> Request {
        let mut request = http::Request::builder().uri(uri);
        for (name, value) in headers {
            request = request.header(*name, HeaderValue::from_bytes(value).unwrap());
        }
        request.body(Bytes::new()).unwrap()
    }

    fn authorized(value: &str) -> Request {
        request("/", &[("authorization", value.as_bytes())])
    }

    #[test]
    fn reads_the_last_query_key_and_the_first_header() {
        let request = request(
            "/?key=first&key=last&other=x",
            &[("x-key", b"one"), ("x-key", b"two")],
        );

        assert_eq!(
            read_api_key(&request, ApiKeyLocation::Query, "key").as_deref(),
            Some("last")
        );
        assert_eq!(
            read_api_key(&request, ApiKeyLocation::Header, "X-Key").as_deref(),
            Some("one")
        );
        assert_eq!(read_api_key(&request, ApiKeyLocation::Query, "Key"), None);
    }

    #[test]
    fn reads_the_last_cookie_of_the_name_from_the_first_cookie_header() {
        let request = request(
            "/",
            &[
                ("cookie", b"session=old; theme=dark; session=\"new\""),
                ("cookie", b"session=ignored"),
            ],
        );

        assert_eq!(
            read_api_key(&request, ApiKeyLocation::Cookie, "session").as_deref(),
            Some("new")
        );
        assert_eq!(
            read_api_key(&self::request("/", &[]), ApiKeyLocation::Cookie, "session"),
            None
        );
    }

    #[test]
    fn an_empty_or_unreadable_key_is_no_key() {
        let request = request("/?key=", &[("x-key", b""), ("cookie", b"session=")]);

        assert_eq!(read_api_key(&request, ApiKeyLocation::Query, "key"), None);
        assert_eq!(
            read_api_key(&request, ApiKeyLocation::Header, "x-key"),
            None
        );
        assert_eq!(
            read_api_key(&request, ApiKeyLocation::Header, "x key"),
            None
        );
        assert_eq!(
            read_api_key(&request, ApiKeyLocation::Cookie, "session"),
            None
        );
    }

    #[test]
    fn decodes_header_keys_as_latin1() {
        let request = request("/", &[("x-key", b"caf\xe9")]);

        assert_eq!(
            read_api_key(&request, ApiKeyLocation::Header, "x-key").as_deref(),
            Some("café")
        );
    }

    #[test]
    fn reads_a_bearer_token_in_any_case_even_an_empty_one() {
        assert_eq!(
            read_bearer_token(&authorized("Bearer abc")).as_deref(),
            Some("abc")
        );
        assert_eq!(
            read_bearer_token(&authorized("bEARER  abc \t")).as_deref(),
            Some("abc")
        );
        assert_eq!(
            read_bearer_token(&authorized("Bearer")).as_deref(),
            Some("")
        );
        assert_eq!(read_bearer_token(&authorized("Basic abc")), None);
        assert_eq!(read_bearer_token(&authorized(" Bearer abc")), None);
        assert_eq!(read_bearer_token(&authorized("")), None);
        assert_eq!(read_bearer_token(&request("/", &[])), None);
    }

    #[test]
    fn reads_bearer_credentials_only_when_both_parts_are_there() {
        let credentials = read_bearer_credentials(&authorized("bearer abc")).unwrap();

        assert_eq!(credentials.scheme, "bearer");
        assert_eq!(credentials.credentials, "abc");
        assert_eq!(read_bearer_credentials(&authorized("Bearer ")), None);
        assert_eq!(read_bearer_credentials(&authorized("Token abc")), None);
        assert_eq!(read_bearer_credentials(&request("/", &[])), None);
    }

    #[test]
    fn reads_basic_credentials_split_at_the_first_colon() {
        // "user:pass:word"
        let credentials = read_basic_credentials(&authorized("Basic dXNlcjpwYXNzOndvcmQ="))
            .unwrap()
            .unwrap();

        assert_eq!(credentials.username, "user");
        assert_eq!(credentials.password, "pass:word");
        assert_eq!(read_basic_credentials(&authorized("Bearer abc")), Ok(None));
        assert_eq!(read_basic_credentials(&request("/", &[])), Ok(None));
    }

    #[test]
    fn refuses_basic_credentials_that_do_not_decode() {
        // not base64, no colon ("user"), and not ASCII ("ü:x")
        for value in ["Basic ???", "Basic dXNlcg==", "Basic w7w6eA==", "Basic"] {
            assert_eq!(
                read_basic_credentials(&authorized(value)),
                Err(InvalidCredentials),
                "{value}"
            );
        }
    }

    #[test]
    fn leaves_secrets_out_of_debug_output() {
        let basic = read_basic_credentials(&authorized("Basic dXNlcjpwYXNz"))
            .unwrap()
            .unwrap();
        let bearer = read_bearer_credentials(&authorized("Bearer secret")).unwrap();

        assert_eq!(
            format!("{basic:?}"),
            r#"HttpBasicCredentials { username: "user", .. }"#
        );
        assert_eq!(
            format!("{bearer:?}"),
            r#"HttpAuthorizationCredentials { scheme: "Bearer", .. }"#
        );
    }
}
