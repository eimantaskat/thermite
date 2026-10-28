use std::borrow::Cow;

use super::percent::percent_decode;

// like Starlette's parse_qsl with keep_blank_values: `+` is a space and `name` alone has an empty value
pub(crate) fn parse_query(query: &str) -> Vec<(Cow<'_, str>, Cow<'_, str>)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode_component(name), decode_component(value))
        })
        .collect()
}

fn decode_component(text: &str) -> Cow<'_, str> {
    if text.contains('+') {
        Cow::Owned(percent_decode(&text.replace('+', " ")).into_owned())
    } else {
        percent_decode(text)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::parse::encode_every_byte;

    fn pairs(query: &str) -> Vec<(String, String)> {
        parse_query(query)
            .into_iter()
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect()
    }

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.to_owned(), value.to_owned())
    }

    #[test]
    fn splits_pairs_in_order_and_keeps_repeated_names() {
        assert_eq!(
            pairs("tag=a&limit=5&tag=b"),
            [pair("tag", "a"), pair("limit", "5"), pair("tag", "b")]
        );
    }

    #[test]
    fn decodes_plus_signs_and_escapes() {
        assert_eq!(
            pairs("q=hot+iron%2Brust&caf%C3%A9=%3D"),
            [pair("q", "hot iron+rust"), pair("café", "=")]
        );
    }

    #[test]
    fn keeps_blank_values_and_skips_empty_pairs() {
        assert_eq!(
            pairs("a=&b&&c=1=2"),
            [pair("a", ""), pair("b", ""), pair("c", "1=2")]
        );
        assert_eq!(pairs(""), Vec::<(String, String)>::new());
    }

    proptest! {
        #[test]
        fn encoded_names_and_values_come_back_unchanged(name in ".+", value in ".*") {
            let query = format!("{}={}", encode_every_byte(&name), encode_every_byte(&value));

            prop_assert_eq!(pairs(&query), [(name, value)]);
        }
    }
}
