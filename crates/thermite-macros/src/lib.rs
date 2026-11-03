#![forbid(unsafe_code)]

mod model;
mod model_attributes;
mod rename_rule;
mod route;
mod security_scheme;
mod serde_attributes;

use proc_macro::TokenStream;
use route::RouteKind;
use syn::{Attribute, DeriveInput, parse_macro_input};

#[proc_macro_derive(Model, attributes(model))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    model::expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

macro_rules! method_attributes {
    ($($name:ident $method:ident),*) => {$(
        #[proc_macro_attribute]
        pub fn $name(path: TokenStream, function: TokenStream) -> TokenStream {
            route::expand(RouteKind::Method(stringify!($method)), path.into(), function.into())
                .unwrap_or_else(syn::Error::into_compile_error)
                .into()
        }
    )*};
}

#[proc_macro_derive(SecurityScheme, attributes(security))]
pub fn derive_security_scheme(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    security_scheme::expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

method_attributes!(get GET, post POST, put PUT, patch PATCH, delete DELETE, head HEAD,
    options OPTIONS, trace TRACE);

#[proc_macro_attribute]
pub fn websocket(path: TokenStream, function: TokenStream) -> TokenStream {
    route::expand(RouteKind::WebSocket, path.into(), function.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn attributes_named<'input>(
    attributes: &'input [Attribute],
    name: &'input str,
) -> impl Iterator<Item = &'input Attribute> {
    attributes
        .iter()
        .filter(move |attribute| attribute.path().is_ident(name))
}
