#![forbid(unsafe_code)]

mod app;
mod arity;
mod background;
mod dependency;
mod error;
mod exception;
mod extract;
mod form;
mod handler;
mod json;
mod lifespan;
mod method_routes;
mod model;
mod rejection;
mod requirements;
mod resource;
mod response;
mod route;
mod router;
mod security;
mod sse;
mod state;
mod streaming;
mod test_client;
mod upload;
mod websocket;

pub use app::App;
pub use bytes::{self, Bytes};
pub use dependency::{Dependency, Depends};
pub use error::Error;
pub use extract::{Cookie, FromRequest, Header, Path, Query};
pub use form::Form;
pub use handler::{Handler, WithResponse};
pub use http::{self, Method, StatusCode};
pub use json::Json;
pub use lifespan::StartupError;
pub use model::{DefaultValue, Model, ToSchema};
pub use rejection::ModelMismatch;
pub use requirements::Requirements;
pub use resource::Resource;
pub use response::IntoResponse;
pub use route::Route;
pub use router::Router;
pub use security::{NotAuthenticated, SecurityScheme};
pub use sse::{EventError, EventSourceResponse, ServerSentEvent};
pub use state::State;
pub use streaming::StreamingResponse;
pub use test_client::{
    TestClient, TestMultipart, TestRequest, TestRequestError, TestResponse, TestStreamingResponse,
    TestWebSocket, TestWebSocketError, TestWebSocketRequest,
};
pub use thermite_core::{
    ApiKeyLocation, Arguments, BackgroundTasks, Body, BodyError, BoxError, CloseCode,
    CorsMiddleware, Field, GZipMiddleware, HttpAuthorizationCredentials, HttpBasicCredentials,
    HttpError, InvalidCredentials, Message, Middleware, Next, OpenApiError, Parameter,
    ParameterLocation, Request, RequestValidationError, Response, ResponseDescription,
    RouteDescription, RouteError, Schema, SchemaLocation, SchemaProblem, SecuritySchemeDescription,
    Server, ServerError, Value, WebSocketDisconnect, read_api_key, read_basic_credentials,
    read_bearer_credentials, read_bearer_token,
};
pub use thermite_macros::{
    Model, SecurityScheme, delete, get, head, options, patch, post, put, trace, websocket,
};
pub use upload::UploadFile;
pub use websocket::{WebSocket, WebSocketConnection, WebSocketHandler, WebSocketOutput};
