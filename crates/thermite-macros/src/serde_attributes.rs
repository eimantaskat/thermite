use quote::ToTokens;
use syn::meta::ParseNestedMeta;
use syn::{Attribute, LitStr, Token, token};

use crate::attributes_named;
use crate::rename_rule::RenameRule;

// attributes that leave the names and shapes serde reads unchanged
const IGNORED_ON_CONTAINER: &[&str] = &[
    "bound",
    "crate",
    "deny_unknown_fields",
    "expecting",
    "rename",
];
const IGNORED_ON_FIELD: &[&str] = &[
    "bound",
    "serialize_with",
    "skip_serializing",
    "skip_serializing_if",
];

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SerdeContainer {
    pub(crate) rename_all: Option<RenameRule>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SerdeField {
    pub(crate) rename: Option<String>,
}

pub(crate) fn container(attributes: &[Attribute]) -> syn::Result<SerdeContainer> {
    let mut container = SerdeContainer::default();
    for attribute in attributes_named(attributes, "serde") {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                if let Some(rule) = deserialize_value(&meta)? {
                    container.rename_all = Some(RenameRule::parse(&rule)?);
                }
                Ok(())
            } else {
                skip_ignored(&meta, IGNORED_ON_CONTAINER)
            }
        })?;
    }
    Ok(container)
}

pub(crate) fn field(attributes: &[Attribute]) -> syn::Result<SerdeField> {
    let mut field = SerdeField::default();
    for attribute in attributes_named(attributes, "serde") {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                if let Some(name) = deserialize_value(&meta)? {
                    field.rename = Some(name.value());
                }
                Ok(())
            } else {
                skip_ignored(&meta, IGNORED_ON_FIELD)
            }
        })?;
    }
    Ok(field)
}

// only the deserialize half matters, since a model describes what requests are read into
fn deserialize_value(meta: &ParseNestedMeta) -> syn::Result<Option<LitStr>> {
    if meta.input.peek(Token![=]) {
        return Ok(Some(meta.value()?.parse()?));
    }
    let mut value = None;
    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("deserialize") {
            value = Some(nested.value()?.parse()?);
            Ok(())
        } else if nested.path.is_ident("serialize") {
            nested.value()?.parse::<LitStr>()?;
            Ok(())
        } else {
            Err(nested.error("expected `serialize` or `deserialize`"))
        }
    })?;
    Ok(value)
}

fn skip_ignored(meta: &ParseNestedMeta, ignored: &[&str]) -> syn::Result<()> {
    if meta.path.is_ident("default") {
        return Err(meta.error(
            "`#[serde(default)]` is not read by `#[derive(Model)]`, use `#[model(default = ...)]`",
        ));
    }
    if !ignored.iter().any(|name| meta.path.is_ident(name)) {
        let path = meta.path.to_token_stream();
        return Err(meta.error(format!(
            "`#[serde({path})]` is not supported by `#[derive(Model)]`"
        )));
    }
    if meta.input.peek(Token![=]) {
        meta.value()?.parse::<LitStr>()?;
    } else if meta.input.peek(token::Paren) {
        meta.parse_nested_meta(|nested| skip_ignored(&nested, &["serialize", "deserialize"]))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use syn::parse_quote;

    use super::*;

    #[test]
    fn follows_the_names_serde_deserializes() {
        let renamed: [Attribute; 2] = [
            parse_quote!(#[serde(rename_all = "camelCase")]),
            parse_quote!(#[serde(rename_all(serialize = "UPPERCASE", deserialize = "kebab-case"))]),
        ];
        let field: [Attribute; 2] = [
            parse_quote!(#[serde(rename = "id")]),
            parse_quote!(#[serde(rename(deserialize = "key", serialize = "out"))]),
        ];

        assert_eq!(
            container(&renamed).unwrap().rename_all,
            Some(RenameRule::Kebab)
        );
        assert_eq!(super::field(&field).unwrap().rename.as_deref(), Some("key"));
    }

    #[test]
    fn a_rename_for_serializing_only_keeps_the_name() {
        let renamed: [Attribute; 1] = [parse_quote!(#[serde(rename(serialize = "out"))])];
        let renamed_all: [Attribute; 1] =
            [parse_quote!(#[serde(rename_all(serialize = "camelCase"))])];

        assert_eq!(field(&renamed).unwrap(), SerdeField::default());
        assert_eq!(container(&renamed_all).unwrap(), SerdeContainer::default());
    }

    #[test]
    fn ignores_attributes_that_keep_what_serde_reads() {
        let on_container: [Attribute; 3] = [
            parse_quote!(#[derive(Debug)]),
            parse_quote!(#[serde(deny_unknown_fields, expecting = "a page", crate = "serde")]),
            parse_quote!(#[serde(bound = "T: Clone", rename(deserialize = "Other"))]),
        ];
        let on_field: [Attribute; 1] = [parse_quote!(
            #[serde(skip_serializing, skip_serializing_if = "Option::is_none",
                serialize_with = "hex", bound(deserialize = "T: Clone"))]
        )];

        assert_eq!(container(&on_container).unwrap(), SerdeContainer::default());
        assert_eq!(field(&on_field).unwrap(), SerdeField::default());
    }

    #[test]
    fn rejects_attributes_that_change_what_serde_reads() {
        let on_container: [Attribute; 1] = [parse_quote!(#[serde(tag = "type")])];
        let on_field: [Attribute; 1] = [parse_quote!(#[serde(flatten)])];
        let nested: [Attribute; 1] = [parse_quote!(#[serde(bound(other = "T: Clone"))])];

        assert_eq!(
            container(&on_container).unwrap_err().to_string(),
            "`#[serde(tag)]` is not supported by `#[derive(Model)]`"
        );
        assert_eq!(
            field(&on_field).unwrap_err().to_string(),
            "`#[serde(flatten)]` is not supported by `#[derive(Model)]`"
        );
        assert_eq!(
            field(&nested).unwrap_err().to_string(),
            "`#[serde(other)]` is not supported by `#[derive(Model)]`"
        );
    }

    // the core answers 422 for a missing field before serde could fill it in or read its alias
    #[test]
    fn rejects_attributes_that_would_read_more_than_the_model_declares() {
        let container_default: [Attribute; 1] = [parse_quote!(#[serde(default)])];
        let field_default: [Attribute; 1] = [parse_quote!(#[serde(default = "ten")])];
        let alias: [Attribute; 1] = [parse_quote!(#[serde(alias = "identifier")])];
        let use_model_default =
            "`#[serde(default)]` is not read by `#[derive(Model)]`, use `#[model(default = ...)]`";

        assert_eq!(
            container(&container_default).unwrap_err().to_string(),
            use_model_default
        );
        assert_eq!(
            field(&field_default).unwrap_err().to_string(),
            use_model_default
        );
        assert_eq!(
            field(&alias).unwrap_err().to_string(),
            "`#[serde(alias)]` is not supported by `#[derive(Model)]`"
        );
    }

    #[test]
    fn rejects_a_rename_for_neither_direction() {
        let renamed: [Attribute; 1] = [parse_quote!(#[serde(rename(both = "id"))])];

        assert_eq!(
            field(&renamed).unwrap_err().to_string(),
            "expected `serialize` or `deserialize`"
        );
    }
}
