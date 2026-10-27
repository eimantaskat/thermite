use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashSet;

use crate::percent::percent_decode;
use crate::route_error::RouteError;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PathParameters(Vec<(String, String)>);

impl PathParameters {
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(parameter, _)| parameter == name)
            .map(|(_, value)| value.as_str())
    }

    #[cfg(test)]
    pub(crate) fn from_pairs<'pair>(
        pairs: impl IntoIterator<Item = (&'pair str, &'pair str)>,
    ) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Parameter(String),
}

impl Segment {
    fn parse(path: &str, text: &str) -> Result<Self, RouteError> {
        if !text.contains(['{', '}']) {
            // request segments are compared decoded, so a literal written in URL form must be too
            return Ok(Self::Literal(percent_decode(text).into_owned()));
        }
        text.strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
            .filter(|name| is_identifier(name))
            .map(|name| Self::Parameter(name.to_owned()))
            .ok_or_else(|| RouteError::InvalidParameter {
                path: path.to_owned(),
                segment: text.to_owned(),
            })
    }

    fn parameter_name(&self) -> Option<&str> {
        match self {
            Self::Literal(_) => None,
            Self::Parameter(name) => Some(name),
        }
    }

    fn is_parameter(&self) -> bool {
        matches!(self, Self::Parameter(_))
    }
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathPattern {
    path: String,
    segments: Vec<Segment>,
}

impl PathPattern {
    pub(crate) fn parse(path: &str) -> Result<Self, RouteError> {
        let Some(rest) = path.strip_prefix('/') else {
            return Err(RouteError::MissingLeadingSlash {
                path: path.to_owned(),
            });
        };
        let segments = rest
            .split('/')
            .map(|text| Segment::parse(path, text))
            .collect::<Result<Vec<_>, _>>()?;

        let mut seen = HashSet::new();
        if let Some(duplicate) = segments
            .iter()
            .filter_map(Segment::parameter_name)
            .find(|name| !seen.insert(*name))
        {
            return Err(RouteError::DuplicateParameter {
                path: path.to_owned(),
                name: duplicate.to_owned(),
            });
        }

        Ok(Self {
            path: path.to_owned(),
            segments,
        })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.path
    }

    pub(crate) fn has_parameter(&self, name: &str) -> bool {
        self.parameter_names().any(|parameter| parameter == name)
    }

    pub(crate) fn parameter_names(&self) -> impl Iterator<Item = &str> {
        self.segments.iter().filter_map(Segment::parameter_name)
    }

    pub(crate) fn matches(&self, segments: &[Cow<'_, str>]) -> bool {
        self.segments.len() == segments.len()
            && self
                .segments
                .iter()
                .zip(segments)
                .all(|(pattern, value)| match pattern {
                    Segment::Literal(literal) => literal == value,
                    Segment::Parameter(_) => !value.is_empty(),
                })
    }

    pub(crate) fn parameters(&self, segments: &[Cow<'_, str>]) -> PathParameters {
        PathParameters(
            self.segments
                .iter()
                .zip(segments)
                .filter_map(|(pattern, value)| {
                    pattern
                        .parameter_name()
                        .map(|name| (name.to_owned(), value.to_string()))
                })
                .collect(),
        )
    }

    pub(crate) fn matches_same_paths_as(&self, other: &Self) -> bool {
        self.segments.len() == other.segments.len()
            && self
                .segments
                .iter()
                .zip(&other.segments)
                .all(|pair| match pair {
                    (Segment::Literal(left), Segment::Literal(right)) => left == right,
                    (Segment::Parameter(_), Segment::Parameter(_)) => true,
                    _ => false,
                })
    }

    // a literal beats a parameter at the first position where the two patterns differ
    pub(crate) fn cmp_specificity(&self, other: &Self) -> Ordering {
        self.segment_kinds().cmp(other.segment_kinds())
    }

    fn segment_kinds(&self) -> impl Iterator<Item = bool> {
        self.segments.iter().map(Segment::is_parameter)
    }
}

// splitting before decoding keeps an encoded `%2F` inside its segment
pub(crate) fn decoded_segments(path: &str) -> Vec<Cow<'_, str>> {
    path.strip_prefix('/')
        .unwrap_or(path)
        .split('/')
        .map(percent_decode)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::percent::encode_every_byte;

    fn pattern(path: &str) -> PathPattern {
        PathPattern::parse(path).unwrap()
    }

    fn match_path(pattern: &PathPattern, path: &str) -> Option<PathParameters> {
        let segments = decoded_segments(path);
        pattern
            .matches(&segments)
            .then(|| pattern.parameters(&segments))
    }

    #[test]
    fn matches_literal_paths_exactly() {
        let items = pattern("/items/all");

        assert!(match_path(&items, "/items/all").is_some());
        assert!(match_path(&items, "/items").is_none());
        assert!(match_path(&items, "/items/all/").is_none());
        assert!(match_path(&items, "/items/other").is_none());
    }

    #[test]
    fn matches_the_root_path() {
        assert!(match_path(&pattern("/"), "/").is_some());
        assert!(match_path(&pattern("/"), "/items").is_none());
    }

    #[test]
    fn captures_parameters_by_name() {
        let parameters = match_path(
            &pattern("/users/{user_id}/items/{item_id}"),
            "/users/7/items/42",
        )
        .unwrap();

        assert_eq!(parameters.get("user_id"), Some("7"));
        assert_eq!(parameters.get("item_id"), Some("42"));
        assert_eq!(parameters.get("missing"), None);
    }

    #[test]
    fn does_not_match_an_empty_parameter() {
        assert!(match_path(&pattern("/items/{id}"), "/items/").is_none());
    }

    #[test]
    fn decodes_parameters_and_literals() {
        let parameters = match_path(&pattern("/café/{name}"), "/caf%C3%A9/a%20b%2Fc").unwrap();

        assert_eq!(parameters.get("name"), Some("a b/c"));
    }

    #[test]
    fn decodes_literals_written_in_url_form() {
        let encoded = pattern("/a%20b/%7Bid%7D");

        assert!(match_path(&encoded, "/a%20b/%7Bid%7D").is_some());
        assert!(match_path(&encoded, "/a%2520b/%7Bid%7D").is_none());
        assert!(encoded.matches_same_paths_as(&pattern("/a b/%7bid%7d")));
    }

    #[test]
    fn rejects_a_path_without_a_leading_slash() {
        assert_eq!(
            PathPattern::parse("items"),
            Err(RouteError::MissingLeadingSlash {
                path: "items".to_owned()
            })
        );
    }

    #[test]
    fn rejects_malformed_parameter_segments() {
        for segment in ["{}", "{id", "id}", "item-{id}", "{1id}", "{id-x}", "{{id}}"] {
            let path = format!("/items/{segment}");
            assert_eq!(
                PathPattern::parse(&path),
                Err(RouteError::InvalidParameter {
                    path: path.clone(),
                    segment: segment.to_owned()
                }),
                "accepted {path}"
            );
        }
    }

    #[test]
    fn rejects_a_parameter_used_twice() {
        assert_eq!(
            PathPattern::parse("/{id}/items/{id}"),
            Err(RouteError::DuplicateParameter {
                path: "/{id}/items/{id}".to_owned(),
                name: "id".to_owned()
            })
        );
    }

    #[test]
    fn patterns_differing_only_in_parameter_names_match_the_same_paths() {
        assert!(pattern("/items/{id}").matches_same_paths_as(&pattern("/items/{name}")));
        assert!(!pattern("/items/{id}").matches_same_paths_as(&pattern("/items/all")));
        assert!(!pattern("/items/{id}").matches_same_paths_as(&pattern("/items/{id}/tags")));
        assert!(!pattern("/items/a").matches_same_paths_as(&pattern("/items/b")));
    }

    #[test]
    fn literal_segments_are_more_specific_than_parameters() {
        assert_eq!(
            pattern("/users/me").cmp_specificity(&pattern("/users/{id}")),
            Ordering::Less
        );
        assert_eq!(
            pattern("/a/{x}").cmp_specificity(&pattern("/{y}/b")),
            Ordering::Less
        );
        assert_eq!(
            pattern("/a/{x}").cmp_specificity(&pattern("/b/{y}")),
            Ordering::Equal
        );
    }

    proptest! {
        #[test]
        fn a_parameter_captures_any_encoded_segment(value in ".+") {
            let path = format!("/items/{}", encode_every_byte(&value));

            let parameters = match_path(&pattern("/items/{id}"), &path);

            prop_assert_eq!(parameters.as_ref().and_then(|found| found.get("id")), Some(value.as_str()));
        }

        #[test]
        fn parsing_any_path_returns_without_panicking(path in "/[a-z{}/_0-9]*") {
            let _ = PathPattern::parse(&path);
        }
    }
}
