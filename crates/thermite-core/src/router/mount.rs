use std::borrow::Cow;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::route_error::RouteError;
use crate::static_files::{StaticFiles, relative_path};

use super::path::decoded_segments;

pub(super) struct Mount {
    path: String,
    // decoded, as request segments are compared; none for a mount at the root
    segments: Vec<String>,
    files: Arc<StaticFiles>,
}

impl Mount {
    pub(super) fn new(path: String, files: StaticFiles) -> Result<Self, RouteError> {
        let trimmed = path.strip_suffix('/').unwrap_or(&path);
        let segments: Vec<String> = if trimmed.is_empty() {
            Vec::new()
        } else {
            decoded_segments(trimmed).map(Cow::into_owned).collect()
        };
        if !path.starts_with('/')
            || path.contains(['{', '}'])
            || segments
                .iter()
                .any(|segment| matches!(segment.as_str(), "" | "." | ".."))
        {
            return Err(RouteError::InvalidMountPath { path });
        }
        if !files.has_directory() {
            return Err(RouteError::StaticDirectory {
                path,
                directory: files.directory().to_owned(),
            });
        }
        Ok(Self {
            path: trimmed.to_owned(),
            segments,
            files: Arc::new(files),
        })
    }

    pub(super) fn path(&self) -> &str {
        &self.path
    }

    pub(super) fn depth(&self) -> usize {
        self.segments.len()
    }

    pub(super) fn has_segments_of(&self, other: &Self) -> bool {
        self.segments == other.segments
    }

    // the file path below the mount, `None` inside it for a path naming no file there
    pub(super) fn find(&self, path: &str) -> Option<(&Arc<StaticFiles>, Option<PathBuf>)> {
        let mut segments = decoded_segments(path);
        self.segments
            .iter()
            .all(|own| {
                segments
                    .next()
                    .is_some_and(|segment| segment == own.as_str())
            })
            .then(|| (&self.files, relative_path(segments)))
    }
}

impl fmt::Debug for Mount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = if self.path.is_empty() {
            "/"
        } else {
            &self.path
        };
        write!(formatter, "{path} -> {}", self.files.directory().display())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn mount(path: &str) -> Result<Mount, RouteError> {
        Mount::new(path.to_owned(), StaticFiles::new("static").check_dir(false))
    }

    #[test]
    fn refuses_a_path_without_a_slash_with_a_parameter_or_an_empty_segment() {
        for path in [
            "static",
            "/{name}",
            "/static//files",
            "",
            "//",
            "/static//",
            "/static/..",
            "/./static",
        ] {
            assert_eq!(
                mount(path).err(),
                Some(RouteError::InvalidMountPath {
                    path: path.to_owned()
                }),
                "{path}"
            );
        }
    }

    #[test]
    fn finds_the_files_below_its_decoded_path_and_nothing_beside_it() {
        let static_files = mount("/static/").unwrap();
        let root = mount("/").unwrap();

        let below = static_files.find("/st%61tic/css/site.css").unwrap();
        assert_eq!(below.1, Some(PathBuf::from("css").join("site.css")));
        assert_eq!(
            static_files.find("/static").unwrap().1,
            Some(PathBuf::new())
        );
        assert!(static_files.find("/statics/site.css").is_none());
        assert_eq!(
            root.find("/site.css").unwrap().1,
            Some(PathBuf::from("site.css"))
        );
        assert_eq!(format!("{root:?}"), "/ -> static");
        assert_eq!(format!("{static_files:?}"), "/static -> static");
    }
}
