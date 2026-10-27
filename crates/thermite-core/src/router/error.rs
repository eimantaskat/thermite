use http::Method;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RouteError {
    #[error("path `{path}` must start with `/`")]
    MissingLeadingSlash { path: String },
    #[error("path `{path}` has an invalid parameter segment `{segment}`")]
    InvalidParameter { path: String, segment: String },
    #[error("path `{path}` uses the parameter `{name}` more than once")]
    DuplicateParameter { path: String, name: String },
    #[error("{method} `{path}` matches the same requests as the route `{existing}`")]
    Conflict {
        method: Method,
        path: String,
        existing: String,
    },
}
