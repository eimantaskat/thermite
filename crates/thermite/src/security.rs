use std::future::{Future, ready};

use thermite_core::{
    Arguments, HttpError, InvalidCredentials, Request, Response, ResponseDescription,
    RouteDescription, SecuritySchemeDescription,
};

use crate::extract::FromRequest;
use crate::response::IntoResponse;

// what `#[derive(SecurityScheme)]` implements, along with FromRequest, on a newtype holding the credentials
pub trait SecurityScheme: Sized + Send {
    fn scheme() -> SecuritySchemeDescription;

    // none when the request carries no credentials for this scheme
    fn authenticate(request: &Request) -> Result<Option<Self>, InvalidCredentials>;

    fn require(request: &Request) -> Result<Self, NotAuthenticated> {
        match Self::authenticate(request) {
            Ok(Some(credentials)) => Ok(credentials),
            Ok(None) | Err(InvalidCredentials) => {
                Err(NotAuthenticated(Self::scheme().not_authenticated()))
            }
        }
    }
}

// FastAPI's 401 for missing credentials, which unlike FastAPI the docs list
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAuthenticated(HttpError);

impl IntoResponse for NotAuthenticated {
    fn describe() -> Vec<ResponseDescription> {
        vec![SecuritySchemeDescription::not_authenticated_response()]
    }

    fn into_response(self) -> Response {
        self.0.into_response()
    }
}

// like `auto_error=False`, missing credentials are none, while basic credentials that do not decode are still refused
impl<Scheme: SecurityScheme> FromRequest for Option<Scheme> {
    type Rejection = HttpError;

    fn describe(description: RouteDescription) -> RouteDescription {
        description.security(Scheme::scheme())
    }

    fn from_request(
        request: &Request,
        _: &Arguments,
    ) -> impl Future<Output = Result<Self, HttpError>> + Send {
        ready(
            Scheme::authenticate(request)
                .map_err(|InvalidCredentials| Scheme::scheme().not_authenticated()),
        )
    }
}
