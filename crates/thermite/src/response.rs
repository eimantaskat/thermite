use std::convert::Infallible;

use http::header::CONTENT_TYPE;
use http::{HeaderValue, StatusCode};
use thermite_core::{Response, ResponseDescription, RouteDescription};

pub(crate) const SUCCESSFUL_RESPONSE: &str = "Successful Response";

pub trait IntoResponse {
    // what the docs list for this type, left empty when its status or body is chosen at runtime
    #[must_use]
    fn describe() -> Vec<ResponseDescription> {
        Vec::new()
    }

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
    fn describe() -> Vec<ResponseDescription> {
        vec![ResponseDescription::new(
            StatusCode::OK,
            SUCCESSFUL_RESPONSE,
        )]
    }

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
    // the status is only known at runtime, so the successes of the body are left undocumented
    fn describe() -> Vec<ResponseDescription> {
        Body::describe()
            .into_iter()
            .filter(|response| !response.status().is_success())
            .collect()
    }

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
    fn describe() -> Vec<ResponseDescription> {
        let mut responses = Success::describe();
        responses.extend(Failure::describe());
        responses
    }

    fn into_response(self) -> Response {
        match self {
            Ok(success) => success.into_response(),
            Err(failure) => failure.into_response(),
        }
    }
}

pub(crate) fn describe_responses<Output: IntoResponse>(
    description: RouteDescription,
) -> RouteDescription {
    Output::describe()
        .into_iter()
        .fold(description, RouteDescription::response)
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
    use bytes::Bytes;
    use thermite_core::Schema;

    use super::*;
    use crate::json::Json;

    struct NotFound;

    impl IntoResponse for NotFound {
        fn describe() -> Vec<ResponseDescription> {
            vec![not_found()]
        }

        fn into_response(self) -> Response {
            StatusCode::NOT_FOUND.into_response()
        }
    }

    fn integer_success() -> ResponseDescription {
        ResponseDescription::new(StatusCode::OK, SUCCESSFUL_RESPONSE).with_schema(Schema::integer())
    }

    fn not_found() -> ResponseDescription {
        ResponseDescription::new(StatusCode::NOT_FOUND, "Item not found")
    }

    #[test]
    fn a_body_decided_at_runtime_documents_nothing() {
        assert_eq!(Response::describe(), []);
        assert_eq!(String::describe(), []);
        assert_eq!(<&str>::describe(), []);
        assert_eq!(StatusCode::describe(), []);
        assert_eq!(Infallible::describe(), []);
    }

    #[test]
    fn documents_unit_as_a_success_without_a_body_and_json_with_its_schema() {
        assert_eq!(
            <()>::describe(),
            [ResponseDescription::new(
                StatusCode::OK,
                SUCCESSFUL_RESPONSE
            )]
        );
        assert_eq!(Json::<i64>::describe(), [integer_success()]);
    }

    #[test]
    fn a_result_documents_both_sides_in_order() {
        assert_eq!(
            Result::<Json<i64>, NotFound>::describe(),
            [integer_success(), not_found()]
        );
    }

    #[test]
    fn a_status_code_in_a_pair_leaves_only_the_failures_of_the_body_documented() {
        assert_eq!(
            <(StatusCode, Result<Json<i64>, NotFound>)>::describe(),
            [not_found()]
        );
    }

    fn parts(response: &Response) -> (StatusCode, Option<&str>, &[u8]) {
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        let body = response.body().as_bytes().map_or(&[][..], AsRef::as_ref);
        (response.status(), content_type, body)
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

        assert_eq!(
            success.into_response().body().as_bytes(),
            Some(&Bytes::from("found"))
        );
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
