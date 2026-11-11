mod file_response;
mod lookup;
mod media_type;
mod range;

use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use http::header::{ALLOW, LOCATION};
use http::{HeaderValue, Method, StatusCode};

use crate::body::Body;
use crate::handler::{Request, Response};
use crate::http_error::HttpError;
use crate::response::internal_server_error;
use file_response::FileResponse;
use lookup::lookup;
pub(crate) use lookup::relative_path;

// Starlette's StaticFiles: the files of a directory, served under the path it is mounted at
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticFiles {
    directory: PathBuf,
    html: bool,
    check_dir: bool,
    follow_symlink: bool,
}

enum Found {
    File(FileResponse),
    // a directory holding an index.html, asked for without the trailing slash its links need
    Directory,
    Nothing,
}

impl StaticFiles {
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            html: false,
            check_dir: true,
            follow_symlink: false,
        }
    }

    // a directory answers its index.html, and a missing file the directory's 404.html
    #[must_use]
    pub fn html(mut self, html: bool) -> Self {
        self.html = html;
        self
    }

    // off for a directory created after the app is built
    #[must_use]
    pub fn check_dir(mut self, check_dir: bool) -> Self {
        self.check_dir = check_dir;
        self
    }

    // on, a symlink in the directory is served wherever it points
    #[must_use]
    pub fn follow_symlink(mut self, follow_symlink: bool) -> Self {
        self.follow_symlink = follow_symlink;
        self
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    // blocking, but asked once when mounted, as Starlette asks when it is built
    pub(crate) fn has_directory(&self) -> bool {
        !self.check_dir || self.directory.is_dir()
    }

    // a path climbing out finds nothing, so in html mode it gets the 404 page as in Starlette
    fn find(&self, relative: Option<&Path>, trailing_slash: bool) -> io::Result<Found> {
        let find = |path: &Path| lookup(&self.directory, path, self.follow_symlink);
        let found = match relative {
            Some(relative) => find(relative)?.map(|found| (relative, found)),
            None => None,
        };
        match found {
            Some((_, (path, metadata))) if metadata.is_file() => {
                return Ok(Found::File(FileResponse::new(
                    path,
                    &metadata,
                    StatusCode::OK,
                )));
            }
            Some((relative, (_, metadata))) if metadata.is_dir() && self.html => {
                if let Some((path, metadata)) = find(&relative.join("index.html"))?
                    && metadata.is_file()
                {
                    return Ok(if trailing_slash {
                        Found::File(FileResponse::new(path, &metadata, StatusCode::OK))
                    } else {
                        Found::Directory
                    });
                }
            }
            _ => {}
        }
        if self.html
            && let Some((path, metadata)) = find(Path::new("404.html"))?
            && metadata.is_file()
        {
            return Ok(Found::File(FileResponse::new(
                path,
                &metadata,
                StatusCode::NOT_FOUND,
            )));
        }
        Ok(Found::Nothing)
    }
}

// `relative` is `None` for a path that names no file inside the directory
pub(crate) async fn respond(
    files: &Arc<StaticFiles>,
    request: &Request,
    relative: Option<PathBuf>,
) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return HttpError::new(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed")
            .with_header(ALLOW, HeaderValue::from_static("GET, HEAD"))
            .into_response();
    }
    let trailing_slash = request.uri().path().ends_with('/');
    let files = Arc::clone(files);
    // the lookup cannot panic, so the task fails only when the runtime shuts down
    let Ok(found) =
        tokio::task::spawn_blocking(move || files.find(relative.as_deref(), trailing_slash)).await
    else {
        return internal_server_error();
    };
    match found {
        Ok(Found::File(file)) => file.respond(request),
        Ok(Found::Directory) => redirect_to_directory(request),
        Ok(Found::Nothing) => not_found(),
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            HttpError::new(StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
        }
        Err(error) => {
            tracing::error!("could not look up a static file: {error}");
            internal_server_error()
        }
    }
}

fn not_found() -> Response {
    HttpError::new(StatusCode::NOT_FOUND, "Not Found").into_response()
}

// a path relative to the host, its leading slashes collapsed, since `//docs/` would name the host `docs`
fn redirect_to_directory(request: &Request) -> Response {
    let uri = request.uri();
    let path = uri.path().trim_start_matches(['/', '\\']);
    let location = match uri.query() {
        Some(query) => format!("/{path}/?{query}"),
        None => format!("/{path}/"),
    };
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
    // a URI path and query are visible ASCII, so this always succeeds
    if let Ok(location) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(LOCATION, location);
    }
    response
}
