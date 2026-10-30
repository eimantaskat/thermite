use std::error::Error;
use std::sync::Arc;

use bytes::Bytes;
use http::header::CONTENT_TYPE;
use http::{HeaderValue, StatusCode};

use crate::handler::Response;

pub(crate) fn json_response(status: StatusCode, body: impl Into<Bytes>) -> Response {
    let mut response = Response::new(body.into());
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

// the error a response answers, kept so that an exception handler can replace the response
#[derive(Clone)]
struct AttachedError(Arc<dyn Error + Send + Sync>);

pub fn attach_error(response: &mut Response, error: Arc<dyn Error + Send + Sync>) {
    response.extensions_mut().insert(AttachedError(error));
}

#[must_use]
pub fn response_error(response: &Response) -> Option<&(dyn Error + Send + Sync + 'static)> {
    response
        .extensions()
        .get::<AttachedError>()
        .map(|attached| attached.0.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_response_answers_no_error_until_one_is_attached() {
        let mut response = json_response(StatusCode::BAD_GATEWAY, "{}");
        let before = response_error(&response).is_none();

        attach_error(&mut response, Arc::new(std::fmt::Error));

        assert!(before);
        let attached = response_error(&response).and_then(|error| error.downcast_ref());
        assert_eq!(attached, Some(&std::fmt::Error));
    }
}
