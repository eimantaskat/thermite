#![forbid(unsafe_code)]

mod app;
mod arity;
mod dependency;
mod error;
mod exception;
mod extract;
mod handler;
mod json;
mod lifespan;
mod method_routes;
mod model;
mod rejection;
mod resource;
mod response;
mod route;
mod router;
mod state;
mod test_client;

pub use app::App;
pub use bytes::{self, Bytes};
pub use dependency::{Dependency, Depends};
pub use error::Error;
pub use extract::{Cookie, FromRequest, Header, Path, Query};
pub use handler::{Handler, WithResponse};
pub use http::{self, Method, StatusCode};
pub use json::Json;
pub use lifespan::StartupError;
pub use model::{Model, ToSchema};
pub use rejection::ModelMismatch;
pub use resource::Resource;
pub use response::IntoResponse;
pub use route::Route;
pub use router::Router;
pub use state::{RequiredState, State};
pub use test_client::{TestClient, TestRequest, TestRequestError, TestResponse};
pub use thermite_core::{
    Arguments, Field, HttpError, Middleware, Next, OpenApiError, Parameter, ParameterLocation,
    Request, RequestValidationError, Response, ResponseDescription, RouteDescription, RouteError,
    Schema, SchemaLocation, SchemaProblem, Server, ServerError, Value,
};
pub use thermite_macros::{Model, delete, get, head, options, patch, post, put, trace};
