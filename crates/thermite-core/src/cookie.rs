use std::borrow::Cow;

// Starlette's cookie_parser: a chunk without `=` has an empty name, and a pair counts unless both sides are empty
pub(crate) fn parse_cookies(header: &str) -> Vec<(&str, Cow<'_, str>)> {
    header
        .split(';')
        .filter_map(|chunk| {
            let (name, value) = chunk.split_once('=').unwrap_or(("", chunk));
            let (name, value) = (strip(name), strip(value));
            (!name.is_empty() || !value.is_empty()).then(|| (name, unquote(value)))
        })
        .collect()
}

// Python's str.strip also treats the separators \x1c to \x1f as whitespace
fn strip(text: &str) -> &str {
    text.trim_matches(|character: char| {
        character.is_whitespace() || ('\x1c'..='\x1f').contains(&character)
    })
}

// Python's http.cookies._unquote: quotes come off, then `\ooo` and `\<character>` escapes are decoded
fn unquote(value: &str) -> Cow<'_, str> {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return Cow::Borrowed(value);
    };
    let mut unquoted = String::with_capacity(inner.len());
    let mut rest = inner;
    while let Some((before, after)) = rest.split_once('\\') {
        unquoted.push_str(before);
        rest = if let Some((character, after)) = octal_escape(after) {
            unquoted.push(character);
            after
        } else {
            match after.chars().next() {
                // Python's regex `.` does not match a newline, so that backslash stays
                Some(character) if character != '\n' => {
                    unquoted.push(character);
                    &after[character.len_utf8()..]
                }
                _ => {
                    unquoted.push('\\');
                    after
                }
            }
        };
    }
    unquoted.push_str(rest);
    Cow::Owned(unquoted)
}

fn octal_escape(text: &str) -> Option<(char, &str)> {
    let digit = |byte: u8, highest: u8| (b'0'..=highest).contains(&byte).then(|| byte - b'0');
    match text.as_bytes() {
        [first, second, third, ..] => {
            let code =
                digit(*first, b'3')? << 6 | digit(*second, b'7')? << 3 | digit(*third, b'7')?;
            Some((char::from(code), &text[3..]))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use proptest::prelude::*;

    use super::*;

    fn cookies(header: &str) -> Vec<(String, String)> {
        parse_cookies(header)
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.into_owned()))
            .collect()
    }

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.to_owned(), value.to_owned())
    }

    #[test]
    fn splits_on_semicolons_and_keeps_everything_after_the_first_equals_sign() {
        assert_eq!(
            cookies("keebler=E=mc2; key:term=value:term;a=b"),
            [
                pair("keebler", "E=mc2"),
                pair("key:term", "value:term"),
                pair("a", "b")
            ]
        );
    }

    #[test]
    fn treats_a_chunk_without_equals_as_a_value_with_an_empty_name() {
        assert_eq!(
            cookies("  =  b  ;  ;  =  ;  c  =  ;  abc"),
            [pair("", "b"), pair("c", ""), pair("", "abc")]
        );
    }

    #[test]
    fn unquotes_values_like_python() {
        assert_eq!(
            cookies(r#"a="Loves"; b="\"q\" \012\\"; c=\012; d=""; e=""#),
            [
                pair("a", "Loves"),
                pair("b", "\"q\" \n\\"),
                pair("c", r"\012"),
                pair("d", ""),
                pair("e", "\"")
            ]
        );
        assert_eq!(unquote("\"a\\\nb\\\""), "a\\\nb\\");
        assert_eq!(unquote("\"\\400\\\u{e9}\""), "400\u{e9}");
    }

    proptest! {
        #[test]
        fn plain_pairs_come_back_in_order(
            pairs in proptest::collection::vec(("[a-z]{1,8}", "[a-z0-9]{0,8}"), 0..8)
        ) {
            let header = pairs
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");

            prop_assert_eq!(cookies(&header), pairs);
        }

        #[test]
        fn quoted_values_with_octal_escapes_come_back_unchanged(value in "[\\x00-\\xff]*") {
            let escaped = value.chars().fold(String::new(), |mut escaped, character| {
                let _ = write!(escaped, "\\{:03o}", u32::from(character));
                escaped
            });

            prop_assert_eq!(cookies(&format!("name=\"{escaped}\"")), [pair("name", &value)]);
        }
    }
}
