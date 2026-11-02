use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{Error, ItemFn, LitStr};

use crate::attributes_named;

pub(crate) fn expand(
    method: &str,
    path: TokenStream,
    function: TokenStream,
) -> syn::Result<TokenStream> {
    let path: LitStr = syn::parse2(path)?;
    let function: ItemFn = syn::parse2(function)?;
    let signature = &function.sig;
    if let Some(receiver) = signature.receiver() {
        return Err(Error::new_spanned(
            receiver,
            "a route handler is a free function, not a method",
        ));
    }
    if !signature.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &signature.generics,
            "a route handler cannot be generic",
        ));
    }
    let name = &signature.ident;
    let visibility = &function.vis;
    let method = Ident::new(method, Span::call_site());
    let conditions: Vec<_> = attributes_named(&function.attrs, "cfg").collect();
    // a braced struct lives only in the type namespace, so it can share the function's name
    Ok(quote! {
        #function

        #(#conditions)*
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        #visibility struct #name {}

        #(#conditions)*
        // a deprecated handler can still be served, and its route is no use of its own
        #[allow(deprecated)]
        impl #name {
            #visibility fn route() -> ::thermite::Route {
                ::thermite::Route::new(::thermite::Method::#method, #path, #name)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn expanded(path: TokenStream, function: TokenStream) -> String {
        expand("GET", path, function).unwrap().to_string()
    }

    fn error(path: TokenStream, function: TokenStream) -> String {
        expand("GET", path, function).unwrap_err().to_string()
    }

    #[test]
    fn keeps_the_function_and_adds_its_route_under_the_same_name() {
        let function = quote! {
            pub(crate) async fn read_item(Path(path): Path<ItemPath>) -> String {
                path.item_id.to_string()
            }
        };

        let expected = quote! {
            pub(crate) async fn read_item(Path(path): Path<ItemPath>) -> String {
                path.item_id.to_string()
            }

            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            pub(crate) struct read_item {}

            #[allow(deprecated)]
            impl read_item {
                pub(crate) fn route() -> ::thermite::Route {
                    ::thermite::Route::new(::thermite::Method::GET, "/items/{item_id}", read_item)
                }
            }
        };
        assert_eq!(
            expanded(quote!("/items/{item_id}"), function),
            expected.to_string()
        );
    }

    #[test]
    fn leaves_out_the_route_wherever_the_function_is_left_out() {
        let function = quote! {
            #[cfg(feature = "admin")]
            #[inline]
            async fn admin() {}
        };

        let expanded = expanded(quote!("/admin"), function);

        assert_eq!(expanded.matches("# [cfg (feature = \"admin\")]").count(), 3);
        assert_eq!(expanded.matches("# [inline]").count(), 1);
    }

    #[test]
    fn rejects_a_path_that_is_not_a_single_string() {
        let function = quote!(
            async fn home() {}
        );

        assert_eq!(
            error(quote!(), function.clone()),
            "unexpected end of input, expected string literal"
        );
        assert_eq!(
            error(quote!(home), function.clone()),
            "expected string literal"
        );
        assert_eq!(error(quote!("/", "/home"), function), "unexpected token");
    }

    #[test]
    fn rejects_anything_but_a_free_function_that_is_not_generic() {
        let method = quote!(
            async fn read(&self) {}
        );
        let generic = quote!(
            async fn read<T>() {}
        );
        let structure = quote!(
            struct Item;
        );

        assert_eq!(
            error(quote!("/"), method),
            "a route handler is a free function, not a method"
        );
        assert_eq!(
            error(quote!("/"), generic),
            "a route handler cannot be generic"
        );
        assert_eq!(error(quote!("/"), structure), "expected `fn`");
    }
}
