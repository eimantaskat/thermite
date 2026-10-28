#![forbid(unsafe_code)]

mod app;
mod extract;
mod handler;

pub use app::App;
pub use extract::FromRequest;
pub use handler::Handler;
pub use http::{self, Method, StatusCode};
pub use thermite_core::{
    Arguments, Field, Parameter, Request, Response, RouteDescription, RouteError, Schema, Server,
    ServerError, Value,
};
