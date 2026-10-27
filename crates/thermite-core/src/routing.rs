mod path;

use std::fmt;

use http::Method;

use crate::route_error::RouteError;

pub(crate) use path::{PathParameters, PathPattern};

// knows methods and paths only, so routing never depends on what is stored per route
pub(crate) struct RouteTable<T> {
    entries: Vec<Entry<T>>,
}

struct Entry<T> {
    method: Method,
    pattern: PathPattern,
    value: T,
}

pub(crate) enum Lookup<'table, T> {
    Found {
        value: &'table T,
        parameters: PathParameters,
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
        }
    }

    pub(crate) fn insert(
        &mut self,
        method: Method,
        pattern: PathPattern,
        value: T,
    ) -> Result<(), RouteError> {
        if let Some(existing) = self
            .entries
            .iter()
            .find(|entry| entry.method == method && entry.pattern.matches_same_paths_as(&pattern))
        {
            return Err(RouteError::Conflict {
                method,
                path: pattern.as_str().to_owned(),
                existing: existing.pattern.as_str().to_owned(),
            });
        }

        let index = self
            .entries
            .partition_point(|entry| entry.pattern.cmp_specificity(&pattern).is_le());
        self.entries.insert(
            index,
            Entry {
                method,
                pattern,
                value,
            },
        );
        Ok(())
    }

    pub(crate) fn lookup(&self, method: &Method, path: &str) -> Lookup<'_, T> {
        let segments = path::decoded_segments(path);
        let matching: Vec<&Entry<T>> = self
            .entries
            .iter()
            .filter(|entry| entry.pattern.matches(&segments))
            .collect();

        // like Starlette, a GET route also answers HEAD, and hyper leaves the body out
        let accepts = |entry: &Entry<T>| {
            entry.method == method || (*method == Method::HEAD && entry.method == Method::GET)
        };
        let found = matching
            .iter()
            .copied()
            .find(|entry| accepts(entry))
            .map(|most_specific| {
                // an explicit HEAD route on the same pattern beats the GET fallback
                matching
                    .iter()
                    .copied()
                    .find(|entry| {
                        entry.method == method
                            && entry.pattern.matches_same_paths_as(&most_specific.pattern)
                    })
                    .unwrap_or(most_specific)
            });

        match found {
            Some(entry) => Lookup::Found {
                value: &entry.value,
                parameters: entry.pattern.parameters(&segments),
            },
            None if matching.is_empty() => Lookup::NotFound,
            None => Lookup::MethodNotAllowed {
                allowed: allowed_methods(&matching),
            },
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

fn allowed_methods<T>(entries: &[&Entry<T>]) -> Vec<Method> {
    let mut allowed = Vec::new();
    for entry in entries {
        let implied = (entry.method == Method::GET).then_some(Method::HEAD);
        for method in std::iter::once(entry.method.clone()).chain(implied) {
            if !allowed.contains(&method) {
                allowed.push(method);
            }
        }
    }
    allowed
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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
    fn debug_output_lists_methods_and_patterns() {
        let table = table(&[(Method::POST, "/items"), (Method::GET, "/items/{id}")]);

        assert_eq!(format!("{table:?}"), "[POST /items, GET /items/{id}]");
    }
}
