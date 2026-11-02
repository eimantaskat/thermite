#![forbid(unsafe_code)]

mod model;
mod model_attributes;
mod rename_rule;
mod route;
mod serde_attributes;

use proc_macro::TokenStream;
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
            route::expand(stringify!($method), path.into(), function.into())
                .unwrap_or_else(syn::Error::into_compile_error)
                .into()
        }
    )*};
}

method_attributes!(get GET, post POST, put PUT, patch PATCH, delete DELETE, head HEAD,
    options OPTIONS, trace TRACE);

fn attributes_named<'input>(
    attributes: &'input [Attribute],
    name: &'input str,
) -> impl Iterator<Item = &'input Attribute> {
    attributes
        .iter()
        .filter(move |attribute| attribute.path().is_ident(name))
}
