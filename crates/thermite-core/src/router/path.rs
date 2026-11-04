use std::borrow::Cow;
use std::collections::HashSet;

use crate::parse::percent_decode;
use crate::route_error::RouteError;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PathParameters<'pattern, 'path>(Vec<(&'pattern str, Cow<'path, str>)>);

impl PathParameters<'_, '_> {
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(parameter, _)| *parameter == name)
            .map(|(_, value)| value.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(name, value)| (*name, value.as_ref()))
    }

    #[cfg(test)]
    pub(crate) fn from_pairs<'pattern>(
        pairs: impl IntoIterator<Item = (&'pattern str, &'pattern str)>,
    ) -> PathParameters<'pattern, 'pattern> {
        PathParameters(
            pairs
                .into_iter()
                .map(|(name, value)| (name, Cow::Borrowed(value)))
                .collect(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Segment {
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

    pub(crate) fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub(crate) fn parameters<'path>(
        &self,
        segments: impl Iterator<Item = Cow<'path, str>>,
    ) -> PathParameters<'_, 'path> {
        PathParameters(
            self.segments
                .iter()
                .zip(segments)
                .filter_map(|(pattern, value)| pattern.parameter_name().map(|name| (name, value)))
                .collect(),
        )
    }
}

// splitting before decoding keeps an encoded `%2F` inside its segment
pub(crate) fn decoded_segments(path: &str) -> impl Iterator<Item = Cow<'_, str>> + Clone {
    path.strip_prefix('/')
        .unwrap_or(path)
        .split('/')
        .map(percent_decode)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

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

    proptest! {
        #[test]
        fn parsing_any_path_returns_without_panicking(path in "/[a-z{}/_0-9]*") {
            let _ = PathPattern::parse(&path);
        }
    }
}
