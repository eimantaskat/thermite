#![forbid(unsafe_code)]

mod handler;
mod server;

pub use handler::{Request, RequestHandler, Response};
pub use server::{Server, ServerError};
