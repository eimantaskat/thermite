use http::header::VARY;
use http::{HeaderMap, HeaderValue};

// as Starlette writes it: the values the response had, then this one, in a single header
pub(super) fn append_vary(headers: &mut HeaderMap, value: &'static str) {
    let mut joined = Vec::new();
    for existing in headers.get_all(VARY) {
        joined.extend_from_slice(existing.as_bytes());
        joined.extend_from_slice(b", ");
    }
    joined.extend_from_slice(value.as_bytes());
    if let Ok(vary) = HeaderValue::from_bytes(&joined) {
        headers.insert(VARY, vary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_every_vary_value_the_response_had_into_one() {
        let mut headers = HeaderMap::new();
        append_vary(&mut headers, "Origin");
        assert_eq!(headers[VARY], "Origin");

        headers.append(VARY, HeaderValue::from_static("Cookie"));
        append_vary(&mut headers, "Accept-Encoding");

        let values: Vec<_> = headers.get_all(VARY).iter().collect();
        assert_eq!(values, ["Origin, Cookie, Accept-Encoding"]);
    }
}
