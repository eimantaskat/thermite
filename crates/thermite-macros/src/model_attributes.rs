use syn::{Attribute, Expr};

use crate::attributes_named;

#[derive(Default)]
pub(crate) struct ModelField {
    pub(crate) default: Option<Expr>,
}

pub(crate) fn field(attributes: &[Attribute]) -> syn::Result<ModelField> {
    let mut field = ModelField::default();
    for attribute in attributes_named(attributes, "model") {
        attribute.parse_nested_meta(|meta| {
            if !meta.path.is_ident("default") {
                return Err(meta.error("expected `default = ...`"));
            }
            if field.default.is_some() {
                return Err(meta.error("the default is already set"));
            }
            field.default = Some(meta.value()?.parse()?);
            Ok(())
        })?;
    }
    Ok(field)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use quote::ToTokens;
    use syn::parse_quote;

    use super::*;

    #[test]
    fn reads_the_default_expression() {
        let attributes: [Attribute; 2] = [
            parse_quote!(#[serde(default)]),
            parse_quote!(#[model(default = Vec::<String>::new())]),
        ];

        let default = field(&attributes).unwrap().default.unwrap();

        assert_eq!(
            default.to_token_stream().to_string(),
            "Vec :: < String > :: new ()"
        );
    }

    // syn parses arrays, references and blocks only with its `full` feature
    #[test]
    fn reads_any_expression_as_the_default() {
        let attributes: [[Attribute; 1]; 3] = [
            [parse_quote!(#[model(default = ["a", "b"])])],
            [parse_quote!(#[model(default = &[1][..])])],
            [parse_quote!(#[model(default = if cfg!(test) { 1 } else { 2 })])],
        ];

        for attributes in attributes {
            assert!(field(&attributes).unwrap().default.is_some());
        }
    }

    #[test]
    fn rejects_unknown_and_repeated_settings() {
        let unknown: [Attribute; 1] = [parse_quote!(#[model(rename = "id")])];
        let repeated: [Attribute; 2] = [
            parse_quote!(#[model(default = 1)]),
            parse_quote!(#[model(default = 2)]),
        ];

        assert_eq!(
            field(&unknown).err().unwrap().to_string(),
            "expected `default = ...`"
        );
        assert_eq!(
            field(&repeated).err().unwrap().to_string(),
            "the default is already set"
        );
    }
}
