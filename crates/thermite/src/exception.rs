use std::error::Error as StdError;
use std::fmt;

use thermite_core::{Response, response_error};

use crate::response::IntoResponse;

type ExceptionHandler =
    Box<dyn Fn(&(dyn StdError + Send + Sync + 'static)) -> Option<Response> + Send + Sync>;

#[derive(Default)]
pub(crate) struct ExceptionHandlers(Vec<ExceptionHandler>);

impl ExceptionHandlers {
    pub(crate) fn add<E, Output>(&mut self, handler: impl Fn(&E) -> Output + Send + Sync + 'static)
    where
        E: StdError + Send + Sync + 'static,
        Output: IntoResponse,
    {
        self.0.push(Box::new(move |error| {
            error
                .downcast_ref::<E>()
                .map(|error| handler(error).into_response())
        }));
    }

    // searched newest first, so a later handler for the same type replaces an earlier one, as in FastAPI
    pub(crate) fn handle(&self, response: Response) -> Response {
        let Some(error) = response_error(&response) else {
            return response;
        };
        if let Some(handled) = self.0.iter().rev().find_map(|handler| handler(error)) {
            return handled;
        }
        if response.status().is_server_error() {
            tracing::error!(%error, "an error reached the client unhandled");
        }
        response
    }
}

impl fmt::Debug for ExceptionHandlers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExceptionHandlers")
            .field("len", &self.0.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use thermite_core::HttpError;

    use super::*;

    #[test]
    fn debug_output_counts_the_handlers() {
        let mut handlers = ExceptionHandlers::default();
        handlers.add(|error: &HttpError| error.status());

        assert_eq!(format!("{handlers:?}"), "ExceptionHandlers { len: 1 }");
    }
}
