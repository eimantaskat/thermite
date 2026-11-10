use syn::{Attribute, Expr, Token};

use crate::attributes_named;

pub(crate) const EXPECTED_DEFAULT: &str = "expected `default` or `default = ...`";

#[derive(Default)]
pub(crate) struct ModelField {
    pub(crate) default: Option<DefaultSetting>,
}

pub(crate) enum DefaultSetting {
    // `#[model(default = ...)]`
    Value(Expr),
    // `#[model(default)]`, the type's own default as `DefaultValue` gives it
    OfType,
}

pub(crate) fn field(attributes: &[Attribute]) -> syn::Result<ModelField> {
    let mut field = ModelField::default();
    for attribute in attributes_named(attributes, "model") {
        attribute.parse_nested_meta(|meta| {
            if !meta.path.is_ident("default") {
                return Err(meta.error(EXPECTED_DEFAULT));
            }
            if field.default.is_some() {
                return Err(meta.error("the default is already set"));
            }
            field.default = Some(if meta.input.peek(Token![=]) {
                DefaultSetting::Value(meta.value()?.parse()?)
            } else {
                DefaultSetting::OfType
            });
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

    fn default_value(attributes: &[Attribute]) -> Option<Expr> {
        match field(attributes).unwrap().default {
            Some(DefaultSetting::Value(expression)) => Some(expression),
            _ => None,
        }
    }

    #[test]
    fn reads_the_default_expression() {
        let attributes: [Attribute; 2] = [
            parse_quote!(#[serde(default)]),
            parse_quote!(#[model(default = Vec::<String>::new())]),
        ];

        let default = default_value(&attributes).unwrap();

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
            assert!(default_value(&attributes).is_some());
        }
    }

    #[test]
    fn reads_a_plain_default_as_the_type_default() {
        let attributes: [Attribute; 1] = [parse_quote!(#[model(default)])];

        let default = field(&attributes).unwrap().default;

        assert!(matches!(default, Some(DefaultSetting::OfType)));
    }

    #[test]
    fn rejects_unknown_and_repeated_settings() {
        let unknown: [Attribute; 1] = [parse_quote!(#[model(rename = "id")])];
        let repeated: [Attribute; 2] = [
            parse_quote!(#[model(default)]),
            parse_quote!(#[model(default = 2)]),
        ];

        assert_eq!(field(&unknown).err().unwrap().to_string(), EXPECTED_DEFAULT);
        assert_eq!(
            field(&repeated).err().unwrap().to_string(),
            "the default is already set"
        );
    }
}
