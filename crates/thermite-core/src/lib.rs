#![forbid(unsafe_code)]

mod arguments;
mod description;
mod handler;
mod http_error;
mod middleware;
mod openapi;
mod parse;
mod response;
mod route_error;
mod router;
mod schema;
mod server;
mod validation;

pub use arguments::Arguments;
pub use description::{Parameter, ParameterLocation, RouteDescription};
pub use handler::{Request, RequestHandler, Response, RouteHandler};
pub use http_error::HttpError;
pub use middleware::{Middleware, MiddlewareStack, Next, Pipeline};
pub use openapi::{Docs, OpenApi, OpenApiError};
pub use response::{attach_error, response_error};
pub use route_error::RouteError;
pub use router::Router;
pub use schema::{Field, Schema};
pub use serde_json::Value;
pub use server::{Server, ServerError};
pub use validation::RequestValidationError;
