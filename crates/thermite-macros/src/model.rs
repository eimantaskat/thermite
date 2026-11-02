use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::{
    Data, DeriveInput, Error, Field, Fields, GenericArgument, PathArguments, Type, parse_quote,
};

use crate::serde_attributes::{self, SerdeContainer};
use crate::{attributes_named, model_attributes};

const NAMED_FIELDS_ONLY: &str = "`Model` can only be derived for structs with named fields";

pub(crate) fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new(input.ident.span(), NAMED_FIELDS_ONLY));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(Error::new(input.ident.span(), NAMED_FIELDS_ONLY));
    };
    if let Some(attribute) = attributes_named(&input.attrs, "model").next() {
        return Err(Error::new_spanned(
            attribute,
            "`#[model(...)]` goes on the fields, not on the struct",
        ));
    }
    let container = serde_attributes::container(&input.attrs)?;
    let declarations = named
        .named
        .iter()
        .map(|field| declare(field, &container))
        .collect::<syn::Result<Vec<_>>>()?;
    // a bound on each type parameter, as serde does, so private field types stay out of the impl
    let mut generics = input.generics.clone();
    for parameter in generics.type_params_mut() {
        parameter.bounds.push(parse_quote!(::thermite::ToSchema));
    }
    let name = &input.ident;
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics ::thermite::Model for #name #type_generics #where_clause {
            fn fields() -> ::std::vec::Vec<::thermite::Field> {
                ::std::vec![#(#declarations),*]
            }
        }
    })
}

fn declare(field: &Field, container: &SerdeContainer) -> syn::Result<TokenStream> {
    let serde = serde_attributes::field(&field.attrs)?;
    let model = model_attributes::field(&field.attrs)?;
    // syn keeps the name optional even in named fields, so this cannot be proved by the types
    let ident = field
        .ident
        .as_ref()
        .ok_or_else(|| Error::new_spanned(field, NAMED_FIELDS_ONLY))?;
    let name = serde.rename.unwrap_or_else(|| {
        let name = ident.unraw().to_string();
        match container.rename_all {
            Some(rule) => rule.apply(&name),
            None => name,
        }
    });
    let optional_type = option_inner(&field.ty);
    let schema_type = optional_type.unwrap_or(&field.ty);
    let presence = match (optional_type, &model.default) {
        (None, None) => TokenStream::new(),
        (Some(_), None) => quote!(.optional()),
        (None, Some(default)) => quote!(.with_default(#default)),
        (Some(_), Some(default)) => {
            return Err(Error::new_spanned(
                default,
                "an `Option` field is already optional, so it takes no default",
            ));
        }
    };
    let schema =
        quote_spanned!(schema_type.span()=> <#schema_type as ::thermite::ToSchema>::schema());
    Ok(quote!(::thermite::Field::new(#name, #schema) #presence))
}

// spotted by name, as serde does, so an alias of Option counts as a required field
fn option_inner(field_type: &Type) -> Option<&Type> {
    let Type::Path(path) = field_type else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    match arguments.args.first() {
        Some(GenericArgument::Type(inner))
            if segment.ident == "Option" && path.qself.is_none() && arguments.args.len() == 1 =>
        {
            Some(inner)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn expanded(input: &DeriveInput) -> String {
        expand(input).unwrap().to_string()
    }

    fn error(input: &DeriveInput) -> String {
        expand(input).unwrap_err().to_string()
    }

    #[test]
    fn declares_each_field_with_its_schema_and_presence() {
        let input = parse_quote! {
            struct Filter {
                r#type: String,
                done: Option<bool>,
                #[model(default = 20)]
                limit: i64,
            }
        };

        let expected = quote! {
            #[automatically_derived]
            impl ::thermite::Model for Filter {
                fn fields() -> ::std::vec::Vec<::thermite::Field> {
                    ::std::vec![
                        ::thermite::Field::new("type", <String as ::thermite::ToSchema>::schema()),
                        ::thermite::Field::new("done", <bool as ::thermite::ToSchema>::schema()).optional(),
                        ::thermite::Field::new("limit", <i64 as ::thermite::ToSchema>::schema()).with_default(20)
                    ]
                }
            }
        };
        assert_eq!(expanded(&input), expected.to_string());
    }

    #[test]
    fn names_fields_as_serde_does() {
        let input = parse_quote! {
            #[serde(rename_all = "camelCase")]
            struct Item {
                item_id: i64,
                #[serde(rename = "Label")]
                display_name: String,
            }
        };

        let expanded = expanded(&input);

        assert!(expanded.contains(r#"Field :: new ("itemId""#), "{expanded}");
        assert!(expanded.contains(r#"Field :: new ("Label""#), "{expanded}");
    }

    #[test]
    fn bounds_each_type_parameter_of_a_generic_model() {
        let input = parse_quote! {
            struct Page<'a, T, U: Copy> where T: Clone {
                items: Vec<T>,
                next: Option<&'a U>,
            }
        };

        let expanded = expanded(&input);

        assert!(
            expanded.contains(
                "impl < 'a , T : :: thermite :: ToSchema , U : Copy + :: thermite :: ToSchema > \
                 :: thermite :: Model for Page < 'a , T , U > where T : Clone"
            ),
            "{expanded}"
        );
    }

    #[test]
    fn only_a_single_argument_option_by_that_name_is_optional() {
        let input = parse_quote! {
            struct Odd {
                vector: Vec<i64>,
                qualified: <Self as Trait>::Option<i64>,
                pair: Option<i64, i64>,
                reference: &'static str,
                lifetime: Option<'static>,
                bare: Option,
            }
        };

        assert!(!expanded(&input).contains("optional"));
    }

    #[test]
    fn rejects_anything_but_a_struct_with_named_fields() {
        let enumeration = parse_quote!(
            enum Choice {
                Yes,
            }
        );
        let tuple = parse_quote!(
            struct Pair(i64, i64);
        );
        let unit = parse_quote!(
            struct Nothing;
        );

        for input in [enumeration, tuple, unit] {
            assert_eq!(error(&input), NAMED_FIELDS_ONLY);
        }
    }

    #[test]
    fn rejects_model_attributes_on_the_struct() {
        let input = parse_quote! {
            #[model(default = 1)]
            struct Item {}
        };

        assert_eq!(
            error(&input),
            "`#[model(...)]` goes on the fields, not on the struct"
        );
    }

    #[test]
    fn rejects_a_default_on_an_option_field() {
        let input = parse_quote! {
            struct Item {
                #[model(default = 1)]
                count: Option<i64>,
            }
        };

        assert_eq!(
            error(&input),
            "an `Option` field is already optional, so it takes no default"
        );
    }

    #[test]
    fn passes_on_attribute_errors() {
        let container = parse_quote! {
            #[serde(transparent)]
            struct Item {}
        };
        let serde_field = parse_quote! {
            struct Item {
                #[serde(skip)]
                count: i64,
            }
        };
        let model_field = parse_quote! {
            struct Item {
                #[model(optional)]
                count: i64,
            }
        };

        assert_eq!(
            error(&container),
            "`#[serde(transparent)]` is not supported by `#[derive(Model)]`"
        );
        assert_eq!(
            error(&serde_field),
            "`#[serde(skip)]` is not supported by `#[derive(Model)]`"
        );
        assert_eq!(error(&model_field), "expected `default = ...`");
    }
}
