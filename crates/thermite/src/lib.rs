#![forbid(unsafe_code)]

mod app;
mod arity;
mod dependency;
mod extract;
mod handler;
mod json;
mod lifespan;
mod method_routes;
mod model;
mod rejection;
mod resource;
mod response;
mod router;
mod state;

pub use app::App;
pub use dependency::{Dependency, Depends};
pub use extract::{Cookie, FromRequest, Header, Path, Query};
pub use handler::Handler;
pub use http::{self, Method, StatusCode};
pub use json::Json;
pub use lifespan::StartupError;
pub use model::{Model, ToSchema};
pub use rejection::ModelMismatch;
pub use resource::Resource;
pub use response::IntoResponse;
pub use router::Router;
pub use state::{RequiredState, State};
pub use thermite_core::{
    Arguments, Field, Middleware, Next, OpenApiError, Parameter, ParameterLocation, Request,
    Response, RouteDescription, RouteError, Schema, Server, ServerError, Value,
};
