use std::convert::Infallible;

use http::header::CONTENT_TYPE;
use http::{HeaderValue, StatusCode};
use thermite_core::Response;

pub trait IntoResponse {
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

// lets an extractor or dependency that cannot fail say so in its Rejection type
impl IntoResponse for Infallible {
    fn into_response(self) -> Response {
        match self {}
    }
}

impl IntoResponse for () {
    fn into_response(self) -> Response {
        Response::default()
    }
}

impl IntoResponse for StatusCode {
    fn into_response(self) -> Response {
        let mut response = Response::default();
        *response.status_mut() = self;
        response
    }
}

impl IntoResponse for &'static str {
    fn into_response(self) -> Response {
        plain_text(Response::new(self.into()))
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response {
        plain_text(Response::new(self.into()))
    }
}

// a body that is not a success, such as a Json that failed to serialize, keeps its own status
impl<Body: IntoResponse> IntoResponse for (StatusCode, Body) {
    fn into_response(self) -> Response {
        let (status, body) = self;
        let mut response = body.into_response();
        if response.status().is_success() {
            *response.status_mut() = status;
        }
        response
    }
}

impl<Success: IntoResponse, Failure: IntoResponse> IntoResponse for Result<Success, Failure> {
    fn into_response(self) -> Response {
        match self {
            Ok(success) => success.into_response(),
            Err(failure) => failure.into_response(),
        }
    }
}

fn plain_text(response: Response) -> Response {
    with_content_type(response, "text/plain; charset=utf-8")
}

pub(crate) fn with_content_type(mut response: Response, content_type: &'static str) -> Response {
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

// the body Starlette sends when an app fails
pub(crate) fn internal_server_error() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(response: &Response) -> (StatusCode, Option<&str>, &[u8]) {
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        (response.status(), content_type, response.body())
    }

    #[test]
    fn sends_strings_as_plain_text() {
        let borrowed = "hot".into_response();
        let owned = String::from("iron").into_response();

        assert_eq!(
            parts(&borrowed),
            (
                StatusCode::OK,
                Some("text/plain; charset=utf-8"),
                &b"hot"[..]
            )
        );
        assert_eq!(
            parts(&owned),
            (
                StatusCode::OK,
                Some("text/plain; charset=utf-8"),
                &b"iron"[..]
            )
        );
    }

    #[test]
    fn sends_an_empty_body_for_unit_and_status_codes() {
        assert_eq!(parts(&().into_response()), (StatusCode::OK, None, &b""[..]));
        assert_eq!(
            parts(&StatusCode::NO_CONTENT.into_response()),
            (StatusCode::NO_CONTENT, None, &b""[..])
        );
    }

    #[test]
    fn a_status_code_in_a_pair_replaces_the_status_of_the_body() {
        let response = (StatusCode::CREATED, "made").into_response();

        assert_eq!(
            parts(&response),
            (
                StatusCode::CREATED,
                Some("text/plain; charset=utf-8"),
                &b"made"[..]
            )
        );
    }

    #[test]
    fn a_status_code_in_a_pair_keeps_a_body_that_is_not_a_success() {
        let failed = (StatusCode::CREATED, internal_server_error()).into_response();
        let missing = (StatusCode::CREATED, Err::<(), _>(StatusCode::NOT_FOUND)).into_response();

        assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_result_answers_with_whichever_side_it_holds() {
        let success: Result<&str, StatusCode> = Ok("found");
        let failure: Result<&str, StatusCode> = Err(StatusCode::NOT_FOUND);

        assert_eq!(success.into_response().body(), "found");
        assert_eq!(failure.into_response().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn passes_a_response_through_unchanged() {
        let response = internal_server_error();
        let expected = parts(&response);

        assert_eq!(
            expected,
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Some("text/plain; charset=utf-8"),
                &b"Internal Server Error"[..]
            )
        );
        assert_eq!(parts(&internal_server_error().into_response()), expected);
    }
}
