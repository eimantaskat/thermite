use std::borrow::Cow;
use std::fs::{self, Metadata};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

// Windows also reads a backslash as a separator, so it splits a segment there as Python's normpath does
const SEPARATORS: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };

// `None` for a path climbing out of the directory or naming no plain file, which is a 404
pub(crate) fn relative_path<'path>(
    segments: impl IntoIterator<Item = Cow<'path, str>>,
) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    let mut depth = 0_usize;
    for segment in segments {
        for piece in segment.split(SEPARATORS) {
            match piece {
                "" | "." => {}
                ".." => {
                    depth = depth.checked_sub(1)?;
                    path.pop();
                }
                piece if is_plain_name(piece) => {
                    depth += 1;
                    path.push(piece);
                }
                _ => return None,
            }
        }
    }
    Some(path)
}

// on Windows, a colon names a drive or a data stream, and trailing dots and spaces are dropped
fn is_plain_name(piece: &str) -> bool {
    !piece.contains('\0')
        && !(cfg!(windows) && (piece.contains(':') || piece.ends_with(['.', ' '])))
}

// blocking: each step asks the file system
pub(super) fn lookup(
    directory: &Path,
    relative: &Path,
    follow_symlink: bool,
) -> io::Result<Option<(PathBuf, Metadata)>> {
    let joined = directory.join(relative);
    let full = if follow_symlink {
        joined
    } else {
        // a symlink may point anywhere, so the target must still be inside the directory
        let (Some(root), Some(full)) = (canonical(directory)?, canonical(&joined)?) else {
            return Ok(None);
        };
        if !full.starts_with(&root) {
            return Ok(None);
        }
        full
    };
    match fs::metadata(&full) {
        Ok(metadata) => Ok(Some((full, metadata))),
        Err(error) if is_missing(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn canonical(path: &Path) -> io::Result<Option<PathBuf>> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(Some(path)),
        Err(error) if is_missing(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

// a name too long or one the system refuses cannot be a file, so it is a 404 as in Starlette
fn is_missing(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::NotFound
            | ErrorKind::NotADirectory
            | ErrorKind::InvalidFilename
            | ErrorKind::InvalidInput
    )
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn relative(path: &str) -> Option<PathBuf> {
        relative_path(path.split('/').map(Cow::Borrowed))
    }

    #[test]
    fn drops_empty_and_dot_segments_and_resolves_dot_dots_inside() {
        assert_eq!(
            relative("css//./site.css"),
            Some(PathBuf::from("css/site.css"))
        );
        assert_eq!(
            relative("css/../js/app.js"),
            Some(PathBuf::from("js/app.js"))
        );
        assert_eq!(relative(""), Some(PathBuf::new()));
    }

    #[test]
    fn refuses_a_path_climbing_out_or_holding_a_null() {
        assert_eq!(relative("../secret.txt"), None);
        assert_eq!(relative("css/../../secret.txt"), None);
        assert_eq!(relative("site\0.css"), None);
    }

    #[test]
    fn splits_a_segment_holding_an_encoded_slash() {
        let segments = [
            Cow::Borrowed("css"),
            Cow::Owned("..%2F..".replace("%2F", "/")),
        ];

        assert_eq!(relative_path(segments), None);
    }

    #[cfg(windows)]
    #[test]
    fn refuses_what_windows_reads_as_a_drive_a_stream_or_a_shortened_name() {
        for path in [
            "C:",
            "C:/secret.txt",
            "site.css::$DATA",
            "site.css.",
            "site.css ",
        ] {
            assert_eq!(relative(path), None, "{path}");
        }
        assert_eq!(relative("css\\..\\..\\secret.txt"), None);
    }

    #[test]
    fn a_missing_directory_finds_nothing() {
        let found = lookup(Path::new("no-such-directory"), Path::new("site.css"), false);

        assert!(matches!(found, Ok(None)));
    }

    proptest! {
        #[test]
        fn a_relative_path_never_climbs_out(path in "[a./\\\\:]{0,24}") {
            if let Some(relative) = relative(&path) {
                prop_assert!(relative.is_relative());
                prop_assert!(relative.components().all(|component| matches!(
                    component,
                    std::path::Component::Normal(_)
                )));
            }
        }
    }
}
