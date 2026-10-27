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
