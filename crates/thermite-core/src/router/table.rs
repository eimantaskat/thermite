use std::borrow::Cow;
use std::fmt;

use http::Method;

use crate::route_error::RouteError;

use super::path::{self, PathParameters, PathPattern, Segment};

// knows methods and paths only, so routing never depends on what is stored per route
pub(crate) struct RouteTable<T> {
    // in registration order; the tree refers to them by index
    entries: Vec<Entry<T>>,
    root: Node,
}

struct Entry<T> {
    method: Method,
    pattern: PathPattern,
    value: T,
}

// one level per path segment, so a lookup only compares segments of routes that can still match
#[derive(Default)]
struct Node {
    // sorted for a binary search
    literals: Vec<(String, Node)>,
    parameter: Option<Box<Node>>,
    // routes whose pattern ends here, which all match the same paths
    entries: Vec<usize>,
}

impl Node {
    fn child(&mut self, segment: &Segment) -> &mut Self {
        match segment {
            Segment::Literal(literal) => {
                let index = match self
                    .literals
                    .binary_search_by(|(existing, _)| existing.as_str().cmp(literal))
                {
                    Ok(index) => index,
                    Err(index) => {
                        self.literals
                            .insert(index, (literal.clone(), Self::default()));
                        index
                    }
                };
                &mut self.literals[index].1
            }
            Segment::Parameter(_) => self.parameter.get_or_insert_default(),
        }
    }

    // a literal is tried before the parameter, so matching patterns come most specific first
    fn find<'node, Found>(
        &'node self,
        segments: &[Cow<'_, str>],
        visit: &mut impl FnMut(&'node [usize]) -> Option<Found>,
    ) -> Option<Found> {
        let Some((segment, rest)) = segments.split_first() else {
            return if self.entries.is_empty() {
                None
            } else {
                visit(&self.entries)
            };
        };
        self.literals
            .binary_search_by(|(literal, _)| literal.as_str().cmp(segment))
            .ok()
            .and_then(|index| self.literals[index].1.find(rest, visit))
            .or_else(|| {
                self.parameter
                    .as_ref()
                    .filter(|_| !segment.is_empty())
                    .and_then(|parameter| parameter.find(rest, visit))
            })
    }
}

pub(crate) enum Lookup<'table, 'path, T> {
    Found {
        value: &'table T,
        parameters: PathParameters<'table, 'path>,
    },
    MethodNotAllowed {
        allowed: Vec<Method>,
    },
    NotFound,
}

impl<T> RouteTable<T> {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
            root: Node::default(),
        }
    }

    pub(crate) fn insert(
        &mut self,
        method: Method,
        pattern: PathPattern,
        value: T,
    ) -> Result<(), RouteError> {
        let node = pattern.segments().iter().fold(&mut self.root, Node::child);
        if let Some(existing) = node
            .entries
            .iter()
            .map(|&index| &self.entries[index])
            .find(|entry| entry.method == method)
        {
            return Err(RouteError::Conflict {
                method,
                path: pattern.as_str().to_owned(),
                existing: existing.pattern.as_str().to_owned(),
            });
        }

        node.entries.push(self.entries.len());
        self.entries.push(Entry {
            method,
            pattern,
            value,
        });
        Ok(())
    }

    fn entries_at<'table>(
        &'table self,
        indices: &'table [usize],
    ) -> impl Iterator<Item = &'table Entry<T>> {
        indices.iter().map(|&index| &self.entries[index])
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Method, &PathPattern, &T)> {
        self.entries
            .iter()
            .map(|entry| (&entry.method, &entry.pattern, &entry.value))
    }

    pub(crate) fn lookup<'path>(&self, method: &Method, path: &'path str) -> Lookup<'_, 'path, T> {
        let segments = path::decoded_segments(path);

        let found = self.root.find(&segments, &mut |indices| {
            // an explicit HEAD route beats the GET fallback, which hyper answers without the body
            self.entries_at(indices)
                .find(|entry| entry.method == method)
                .or_else(|| {
                    self.entries_at(indices)
                        .find(|entry| *method == Method::HEAD && entry.method == Method::GET)
                })
        });
        if let Some(entry) = found {
            return Lookup::Found {
                value: &entry.value,
                parameters: entry.pattern.parameters(&segments),
            };
        }

        let mut allowed = Vec::new();
        self.root.find(&segments, &mut |indices| {
            self.entries_at(indices)
                .for_each(|entry| allow(&mut allowed, &entry.method));
            None::<()>
        });
        if allowed.is_empty() {
            Lookup::NotFound
        } else {
            Lookup::MethodNotAllowed { allowed }
        }
    }
}

impl<T> fmt::Debug for RouteTable<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_list()
            .entries(self.entries.iter().map(|entry| {
                fmt::from_fn(move |formatter| {
                    write!(formatter, "{} {}", entry.method, entry.pattern.as_str())
                })
            }))
            .finish()
    }
}

// like Starlette, a GET route also answers HEAD
fn allow(allowed: &mut Vec<Method>, method: &Method) {
    let implied = (*method == Method::GET).then_some(Method::HEAD);
    for method in std::iter::once(method.clone()).chain(implied) {
        if !allowed.contains(&method) {
            allowed.push(method);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::collections::HashMap;

    use proptest::prelude::*;

    use super::*;
    use crate::parse::encode_every_byte;

    fn table(routes: &[(Method, &str)]) -> RouteTable<String> {
        let mut table = RouteTable::new();
        for (method, path) in routes {
            table
                .insert(
                    method.clone(),
                    PathPattern::parse(path).unwrap(),
                    format!("{method} {path}"),
                )
                .unwrap();
        }
        table
    }

    fn found(table: &RouteTable<String>, method: &Method, path: &str) -> Option<String> {
        match table.lookup(method, path) {
            Lookup::Found { value, .. } => Some(value.clone()),
            _ => None,
        }
    }

    fn match_path(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
        match table(&[(Method::GET, pattern)]).lookup(&Method::GET, path) {
            Lookup::Found { parameters, .. } => Some(
                parameters
                    .iter()
                    .map(|(name, value)| (name.to_owned(), value.to_owned()))
                    .collect(),
            ),
            _ => None,
        }
    }

    #[test]
    fn matches_literal_paths_exactly() {
        assert!(match_path("/items/all", "/items/all").is_some());
        assert!(match_path("/items/all", "/items").is_none());
        assert!(match_path("/items/all", "/items/all/").is_none());
        assert!(match_path("/items/all", "/items/other").is_none());
    }

    #[test]
    fn matches_the_root_path() {
        assert!(match_path("/", "/").is_some());
        assert!(match_path("/", "/items").is_none());
    }

    #[test]
    fn captures_parameters_by_name() {
        let parameters =
            match_path("/users/{user_id}/items/{item_id}", "/users/7/items/42").unwrap();

        assert_eq!(parameters.get("user_id").map(String::as_str), Some("7"));
        assert_eq!(parameters.get("item_id").map(String::as_str), Some("42"));
        assert_eq!(parameters.get("missing"), None);
    }

    #[test]
    fn does_not_match_an_empty_parameter() {
        assert!(match_path("/items/{id}", "/items/").is_none());
    }

    #[test]
    fn decodes_parameters_and_literals() {
        let parameters = match_path("/café/{name}", "/caf%C3%A9/a%20b%2Fc").unwrap();

        assert_eq!(parameters.get("name").map(String::as_str), Some("a b/c"));
    }

    #[test]
    fn decodes_literals_written_in_url_form() {
        assert!(match_path("/a%20b/%7Bid%7D", "/a%20b/%7Bid%7D").is_some());
        assert!(match_path("/a%20b/%7Bid%7D", "/a%2520b/%7Bid%7D").is_none());

        let mut table = table(&[(Method::GET, "/a%20b/%7Bid%7D")]);
        let same = table.insert(
            Method::GET,
            PathPattern::parse("/a b/%7bid%7d").unwrap(),
            "again".to_owned(),
        );
        assert!(matches!(same, Err(RouteError::Conflict { .. })));
    }

    #[test]
    fn prefers_a_literal_segment_at_the_first_difference() {
        let table = table(&[(Method::GET, "/{first}/b"), (Method::GET, "/a/{second}")]);

        assert_eq!(
            found(&table, &Method::GET, "/a/b").as_deref(),
            Some("GET /a/{second}")
        );
        assert_eq!(
            found(&table, &Method::GET, "/x/b").as_deref(),
            Some("GET /{first}/b")
        );
    }

    #[test]
    fn falls_back_to_a_parameter_when_the_literal_branch_has_no_match() {
        let table = table(&[(Method::GET, "/a/b"), (Method::GET, "/{first}/c")]);

        assert_eq!(
            found(&table, &Method::GET, "/a/c").as_deref(),
            Some("GET /{first}/c")
        );
    }

    #[test]
    fn an_explicit_head_route_beats_the_get_fallback() {
        let table = table(&[(Method::GET, "/items"), (Method::HEAD, "/items")]);

        assert_eq!(
            found(&table, &Method::HEAD, "/items").as_deref(),
            Some("HEAD /items")
        );
        assert_eq!(
            found(&table, &Method::GET, "/items").as_deref(),
            Some("GET /items")
        );
    }

    #[test]
    fn head_goes_to_the_most_specific_route_even_through_the_get_fallback() {
        let table = table(&[(Method::HEAD, "/items/{id}"), (Method::GET, "/items/all")]);

        assert_eq!(
            found(&table, &Method::HEAD, "/items/all").as_deref(),
            Some("GET /items/all")
        );
        assert_eq!(
            found(&table, &Method::HEAD, "/items/5").as_deref(),
            Some("HEAD /items/{id}")
        );
    }

    #[test]
    fn passes_the_decoded_parameters_of_the_match_along() {
        let table = table(&[(Method::GET, "/items/{id}")]);

        assert!(matches!(
            table.lookup(&Method::GET, "/items/a%20b"),
            Lookup::Found { parameters, .. } if parameters.get("id") == Some("a b")
        ));
    }

    #[test]
    fn lists_get_once_and_head_once_in_allowed_methods() {
        let table = table(&[
            (Method::GET, "/items/{id}"),
            (Method::HEAD, "/items/{id}"),
            (Method::GET, "/items/all"),
        ]);

        assert!(matches!(
            table.lookup(&Method::POST, "/items/all"),
            Lookup::MethodNotAllowed { allowed } if allowed == [Method::GET, Method::HEAD]
        ));
        assert!(matches!(
            table.lookup(&Method::GET, "/users"),
            Lookup::NotFound
        ));
    }

    #[test]
    fn rejects_a_pattern_matching_the_same_requests_for_the_same_method() {
        let mut table = table(&[
            (Method::GET, "/items/{id}"),
            (Method::DELETE, "/items/{id}"),
        ]);

        let conflict = table.insert(
            Method::GET,
            PathPattern::parse("/items/{name}").unwrap(),
            "again".to_owned(),
        );

        assert_eq!(
            conflict,
            Err(RouteError::Conflict {
                method: Method::GET,
                path: "/items/{name}".to_owned(),
                existing: "/items/{id}".to_owned()
            })
        );
    }

    #[test]
    fn accepts_patterns_differing_in_a_literal_a_parameter_or_their_length() {
        let paths = [
            "/items/{id}",
            "/items/all",
            "/items/{id}/tags",
            "/items/a",
            "/items/b",
        ];
        let table = table(&paths.map(|path| (Method::GET, path)));

        for (pattern, request) in paths.iter().zip([
            "/items/5",
            "/items/all",
            "/items/5/tags",
            "/items/a",
            "/items/b",
        ]) {
            assert_eq!(
                found(&table, &Method::GET, request),
                Some(format!("GET {pattern}"))
            );
        }
    }

    #[test]
    fn iterates_in_registration_order_whatever_the_specificity() {
        let table = table(&[
            (Method::GET, "/items/{id}"),
            (Method::POST, "/users"),
            (Method::GET, "/items/all"),
        ]);

        let paths: Vec<&str> = table
            .iter()
            .map(|(_, pattern, _)| pattern.as_str())
            .collect();

        assert_eq!(paths, ["/items/{id}", "/users", "/items/all"]);
    }

    #[test]
    fn debug_output_lists_methods_and_patterns() {
        let table = table(&[(Method::POST, "/items"), (Method::GET, "/items/{id}")]);

        assert_eq!(format!("{table:?}"), "[POST /items, GET /items/{id}]");
    }

    fn pattern_matches(pattern: &PathPattern, segments: &[Cow<'_, str>]) -> bool {
        pattern.segments().len() == segments.len()
            && pattern
                .segments()
                .iter()
                .zip(segments)
                .all(|(segment, value)| match segment {
                    Segment::Literal(literal) => literal == value,
                    Segment::Parameter(_) => !value.is_empty(),
                })
    }

    fn match_the_same_paths(left: &PathPattern, right: &PathPattern) -> bool {
        left.segments().len() == right.segments().len()
            && left
                .segments()
                .iter()
                .zip(right.segments())
                .all(|pair| match pair {
                    (Segment::Literal(left), Segment::Literal(right)) => left == right,
                    (Segment::Parameter(_), Segment::Parameter(_)) => true,
                    _ => false,
                })
    }

    // what the tree replaced: every route in order of specificity, then the first that accepts
    fn scanned(routes: &[(Method, PathPattern)], method: &Method, path: &str) -> String {
        let mut sorted: Vec<&(Method, PathPattern)> = routes.iter().collect();
        sorted.sort_by_key(|(_, pattern)| {
            pattern
                .segments()
                .iter()
                .map(|segment| matches!(segment, Segment::Parameter(_)))
                .collect::<Vec<_>>()
        });
        let segments = path::decoded_segments(path);
        let matching: Vec<_> = sorted
            .into_iter()
            .filter(|(_, pattern)| pattern_matches(pattern, &segments))
            .collect();

        let accepts = |candidate: &Method| {
            candidate == method || (*method == Method::HEAD && *candidate == Method::GET)
        };
        let found = matching
            .iter()
            .find(|(candidate, _)| accepts(candidate))
            .map(|most_specific| {
                matching
                    .iter()
                    .find(|(candidate, pattern)| {
                        candidate == method && match_the_same_paths(pattern, &most_specific.1)
                    })
                    .unwrap_or(most_specific)
            });
        match found {
            Some((candidate, pattern)) => format!(
                "{candidate} {} {:?}",
                pattern.as_str(),
                pattern.parameters(&segments)
            ),
            None if matching.is_empty() => "404".to_owned(),
            None => {
                let mut allowed = Vec::new();
                for (candidate, _) in matching {
                    allow(&mut allowed, candidate);
                }
                format!("405 {allowed:?}")
            }
        }
    }

    fn looked_up(table: &RouteTable<String>, method: &Method, path: &str) -> String {
        match table.lookup(method, path) {
            Lookup::Found { value, parameters } => format!("{value} {parameters:?}"),
            Lookup::MethodNotAllowed { allowed } => format!("405 {allowed:?}"),
            Lookup::NotFound => "404".to_owned(),
        }
    }

    fn methods() -> impl Strategy<Value = Method> {
        prop::sample::select(vec![Method::GET, Method::HEAD, Method::POST])
    }

    fn patterns() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(vec!["a", "b", "", "{p}"]), 1..=3).prop_map(
            |segments| {
                let named = segments.iter().enumerate().map(|(index, segment)| {
                    if *segment == "{p}" {
                        format!("{{p{index}}}")
                    } else {
                        (*segment).to_owned()
                    }
                });
                format!("/{}", named.collect::<Vec<_>>().join("/"))
            },
        )
    }

    fn paths() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(vec!["a", "b", "", "c"]), 1..=3)
            .prop_map(|segments| format!("/{}", segments.join("/")))
    }

    proptest! {
        #[test]
        fn a_parameter_captures_any_encoded_segment(value in ".+") {
            let path = format!("/items/{}", encode_every_byte(&value));

            let parameters = match_path("/items/{id}", &path);

            prop_assert_eq!(parameters.as_ref().and_then(|found| found.get("id")), Some(&value));
        }

        #[test]
        fn finds_what_scanning_every_route_by_specificity_finds(
            routes in prop::collection::vec((methods(), patterns()), 0..12),
            method in methods(),
            path in paths(),
        ) {
            let mut table = RouteTable::new();
            let mut accepted = Vec::new();
            for (route_method, text) in routes {
                let pattern = PathPattern::parse(&text).unwrap();
                let value = format!("{route_method} {text}");
                if table.insert(route_method.clone(), pattern.clone(), value).is_ok() {
                    accepted.push((route_method, pattern));
                }
            }

            prop_assert_eq!(looked_up(&table, &method, &path), scanned(&accepted, &method, &path));
        }
    }
}
