use std::borrow::Cow;

// lenient like Python's urllib.parse.unquote: bad escapes stay as they are, bad UTF-8 becomes U+FFFD
pub(crate) fn percent_decode(text: &str) -> Cow<'_, str> {
    if !text.contains('%') {
        return Cow::Borrowed(text);
    }
    let mut decoded = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%'
            && let [high, low, after @ ..] = tail
            && let (Some(high), Some(low)) = (hex_value(*high), hex_value(*low))
        {
            decoded.push((high << 4) | low);
            rest = after;
        } else {
            decoded.push(byte);
            rest = tail;
        }
    }
    Cow::Owned(String::from_utf8_lossy(&decoded).into_owned())
}

fn hex_value(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

#[cfg(test)]
pub(crate) fn encode_every_byte(text: &str) -> String {
    use std::fmt::Write;

    text.bytes().fold(String::new(), |mut encoded, byte| {
        let _ = write!(encoded, "%{byte:02X}");
        encoded
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn keeps_malformed_escapes_as_they_are() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
        assert_eq!(percent_decode("%+1"), "%+1");
        assert_eq!(percent_decode("%FF"), "\u{FFFD}");
    }

    proptest! {
        #[test]
        fn decoding_reverses_percent_encoding(text in any::<String>()) {
            let encoded = encode_every_byte(&text);
            prop_assert_eq!(percent_decode(&encoded), text);
        }

        #[test]
        fn text_without_escapes_is_borrowed_unchanged(text in "[^%]*") {
            prop_assert!(matches!(percent_decode(&text), Cow::Borrowed(borrowed) if borrowed == text));
        }
    }
}
