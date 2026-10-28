use thermite_core::{Arguments, Request, Response, RouteDescription};

use crate::arity::for_each_arity;
use crate::extract::FromRequest;
use crate::response::IntoResponse;

macro_rules! impl_from_request_for_tuple {
    ($($extractor:ident),*) => {
        impl<$($extractor: FromRequest),*> FromRequest for ($($extractor,)*) {
            type Rejection = Response;

            fn describe(description: RouteDescription) -> RouteDescription {
                $(let description = $extractor::describe(description);)*
                description
            }

            // each value is named after its type parameter, and the empty tuple reads and awaits nothing
            #[allow(non_snake_case, unused_variables, clippy::unused_async_trait_impl)]
            async fn from_request(request: &Request, arguments: &Arguments) -> Result<Self, Response> {
                $(
                    let $extractor = $extractor::from_request(request, arguments)
                        .await
                        .map_err(IntoResponse::into_response)?;
                )*
                Ok(($($extractor,)*))
            }
        }
    };
}

for_each_arity!(impl_from_request_for_tuple);
