use std::borrow::Cow;

// Starlette decodes header values as latin-1, so every byte is one character and nothing fails
pub(crate) fn decode_latin1(bytes: &[u8]) -> Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(text) if text.is_ascii() => Cow::Borrowed(text),
        _ => Cow::Owned(bytes.iter().map(|&byte| char::from(byte)).collect()),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn borrows_ascii_and_maps_other_bytes_to_their_code_points() {
        assert!(matches!(
            decode_latin1(b"Bearer x"),
            Cow::Borrowed("Bearer x")
        ));
        assert_eq!(decode_latin1(b"caf\xe9"), "café");
        assert_eq!(decode_latin1("café".as_bytes()), "cafÃ©");
    }

    proptest! {
        #[test]
        fn keeps_one_character_per_byte(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let decoded: Vec<u32> = decode_latin1(&bytes).chars().map(u32::from).collect();

            prop_assert_eq!(decoded, bytes.into_iter().map(u32::from).collect::<Vec<_>>());
        }
    }
}
