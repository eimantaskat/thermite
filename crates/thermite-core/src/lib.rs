#![forbid(unsafe_code)]

mod arguments;
mod background;
mod body;
mod decoder;
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
mod security;
mod server;
mod upload;
mod validation;
mod websocket;

pub use arguments::Arguments;
pub use background::{BackgroundTasks, attach_background_tasks};
pub use body::{Body, BodyError, BoxError};
pub use decoder::BodyDecoder;
pub use description::{Parameter, ParameterLocation, ResponseDescription, RouteDescription};
pub use handler::{Request, RequestHandler, Response, RouteHandler};
pub use http_error::HttpError;
pub use middleware::{CorsMiddleware, GZipMiddleware, Middleware, MiddlewareStack, Next, Pipeline};
pub use openapi::{Docs, OpenApi, OpenApiError};
pub use response::{attach_error, response_error};
pub use route_error::{RouteError, SchemaLocation, SchemaProblem};
pub use router::Router;
pub use schema::{Field, Schema};
pub use security::{
    ApiKeyLocation, HttpAuthorizationCredentials, HttpBasicCredentials, InvalidCredentials,
    SecuritySchemeDescription, read_api_key, read_basic_credentials, read_bearer_credentials,
    read_bearer_token,
};
pub use serde_json::Value;
pub use server::{InProcessServer, InProcessWebSocket, Server, ServerError, WebSocketHandshake};
pub use upload::{UploadedFile, UploadedFiles};
pub use validation::RequestValidationError;
pub use websocket::{
    CloseCode, Message, WebSocket, WebSocketConnection, WebSocketDisconnect, WebSocketHandler,
};
