use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::meta::ParseNestedMeta;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Error, Fields, Ident, LitStr, Token};

use crate::attributes_named;

const NEWTYPE_ONLY: &str = "`SecurityScheme` can only be derived for a struct with one unnamed field, like `struct Token(String)`";
const SCHEMES: &str = "`api_key`, `http_basic`, `http_bearer` or `oauth2_password`";

pub(crate) fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let field_type = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => &fields.unnamed[0].ty,
            _ => return Err(Error::new(name.span(), NEWTYPE_ONLY)),
        },
        _ => return Err(Error::new(name.span(), NEWTYPE_ONLY)),
    };
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "`SecurityScheme` cannot be derived for a generic struct",
        ));
    }
    let settings = Settings::read(input)?;
    let kind = settings.kind(name)?;
    let mut scheme = match &kind {
        Kind::ApiKey { location, name } => {
            quote!(::thermite::SecuritySchemeDescription::api_key(#location, #name))
        }
        Kind::HttpBasic => quote!(::thermite::SecuritySchemeDescription::http_basic()),
        Kind::HttpBearer => quote!(::thermite::SecuritySchemeDescription::http_bearer()),
        Kind::OAuth2Password { token_url } => {
            quote!(::thermite::SecuritySchemeDescription::oauth2_password(#token_url))
        }
    };
    if let Some(scheme_name) = &settings.scheme_name {
        scheme.extend(quote!(.with_scheme_name(#scheme_name)));
    }
    if let Some(description) = &settings.description {
        scheme.extend(quote!(.with_description(#description)));
    }
    // spanned to the field, so credentials of the wrong type are reported there
    let wrap = quote_spanned!(field_type.span()=> Self(credentials));
    let authenticate = match &kind {
        Kind::ApiKey { location, name } => quote! {
            ::std::result::Result::Ok(
                ::thermite::read_api_key(request, #location, #name).map(|credentials| #wrap)
            )
        },
        Kind::HttpBasic => quote! {
            ::thermite::read_basic_credentials(request)
                .map(|found| found.map(|credentials| #wrap))
        },
        Kind::HttpBearer => quote! {
            ::std::result::Result::Ok(
                ::thermite::read_bearer_credentials(request).map(|credentials| #wrap)
            )
        },
        Kind::OAuth2Password { .. } => quote! {
            ::std::result::Result::Ok(
                ::thermite::read_bearer_token(request).map(|credentials| #wrap)
            )
        },
    };
    Ok(quote! {
        #[automatically_derived]
        impl ::thermite::SecurityScheme for #name {
            fn scheme() -> ::thermite::SecuritySchemeDescription {
                #scheme
            }

            fn authenticate(
                request: &::thermite::Request,
            ) -> ::std::result::Result<::std::option::Option<Self>, ::thermite::InvalidCredentials> {
                #authenticate
            }
        }

        #[automatically_derived]
        impl ::thermite::FromRequest for #name {
            type Rejection = ::thermite::NotAuthenticated;

            fn describe(description: ::thermite::RouteDescription) -> ::thermite::RouteDescription {
                description.security(<Self as ::thermite::SecurityScheme>::scheme())
            }

            fn from_request(
                request: &::thermite::Request,
                _: &::thermite::Arguments,
            ) -> impl ::std::future::Future<
                Output = ::std::result::Result<Self, ::thermite::NotAuthenticated>,
            > + ::std::marker::Send {
                ::std::future::ready(<Self as ::thermite::SecurityScheme>::require(request))
            }
        }
    })
}

enum Kind {
    ApiKey { location: TokenStream, name: LitStr },
    HttpBasic,
    HttpBearer,
    OAuth2Password { token_url: LitStr },
}

// each setting keeps the name it was written with, to point errors at it
#[derive(Default)]
struct Settings {
    scheme: Option<Ident>,
    // with the ApiKeyLocation it stands for
    location: Option<(Ident, TokenStream, LitStr)>,
    token_url: Option<(Ident, LitStr)>,
    scheme_name: Option<LitStr>,
    description: Option<LitStr>,
}

impl Settings {
    fn read(input: &DeriveInput) -> syn::Result<Self> {
        let mut settings = Self::default();
        for attribute in attributes_named(&input.attrs, "security") {
            attribute.parse_nested_meta(|meta| settings.read_one(&meta))?;
        }
        Ok(settings)
    }

    fn read_one(&mut self, meta: &ParseNestedMeta<'_>) -> syn::Result<()> {
        let Some(ident) = meta.path.get_ident().cloned() else {
            return Err(meta.error(unknown_setting()));
        };
        match ident.to_string().as_str() {
            "api_key" | "http_basic" | "http_bearer" | "oauth2_password" => {
                if meta.input.peek(Token![=]) {
                    return Err(meta.error(format!("`{ident}` takes no value")));
                }
                if let Some(scheme) = &self.scheme {
                    return Err(meta.error(format!("the scheme is already `{scheme}`")));
                }
                self.scheme = Some(ident);
            }
            "header" | "query" | "cookie" => {
                if let Some((location, ..)) = &self.location {
                    return Err(meta.error(format!("the key is already read from the {location}")));
                }
                let variant = match ident.to_string().as_str() {
                    "header" => quote!(Header),
                    "query" => quote!(Query),
                    _ => quote!(Cookie),
                };
                let location = quote!(::thermite::ApiKeyLocation::#variant);
                self.location = Some((ident, location, meta.value()?.parse()?));
            }
            "token_url" => set_once(&mut self.token_url, meta, |value| (ident, value))?,
            "scheme_name" => set_once(&mut self.scheme_name, meta, |value| value)?,
            "description" => set_once(&mut self.description, meta, |value| value)?,
            _ => return Err(meta.error(unknown_setting())),
        }
        Ok(())
    }

    fn kind(&self, target: &Ident) -> syn::Result<Kind> {
        let Some(scheme) = &self.scheme else {
            return Err(Error::new(
                target.span(),
                format!("add `#[security(...)]` naming a scheme: {SCHEMES}"),
            ));
        };
        let is_api_key = scheme == "api_key";
        if let Some((location, ..)) = self.location.as_ref().filter(|_| !is_api_key) {
            return Err(Error::new(
                location.span(),
                format!("`{location}` is only for `api_key`"),
            ));
        }
        let is_oauth2 = scheme == "oauth2_password";
        if let Some((token_url, _)) = self.token_url.as_ref().filter(|_| !is_oauth2) {
            return Err(Error::new(
                token_url.span(),
                "`token_url` is only for `oauth2_password`",
            ));
        }
        match scheme.to_string().as_str() {
            "api_key" => {
                let (_, location, name) = self.location.as_ref().ok_or_else(|| {
                    Error::new(
                        scheme.span(),
                        "`api_key` needs `header`, `query` or `cookie` with the key's name",
                    )
                })?;
                Ok(Kind::ApiKey {
                    location: location.clone(),
                    name: name.clone(),
                })
            }
            "http_basic" => Ok(Kind::HttpBasic),
            "http_bearer" => Ok(Kind::HttpBearer),
            _ => {
                let (_, token_url) = self.token_url.as_ref().ok_or_else(|| {
                    Error::new(scheme.span(), "`oauth2_password` needs `token_url`")
                })?;
                Ok(Kind::OAuth2Password {
                    token_url: token_url.clone(),
                })
            }
        }
    }
}

fn set_once<T>(
    slot: &mut Option<T>,
    meta: &ParseNestedMeta<'_>,
    make: impl FnOnce(LitStr) -> T,
) -> syn::Result<()> {
    if slot.is_some() {
        let name = meta
            .path
            .get_ident()
            .map(Ident::to_string)
            .unwrap_or_default();
        return Err(meta.error(format!("`{name}` is already set")));
    }
    *slot = Some(make(meta.value()?.parse()?));
    Ok(())
}

fn unknown_setting() -> String {
    format!(
        "expected a scheme, {SCHEMES}, or one of `header`, `query`, `cookie`, `token_url`, `scheme_name` and `description`"
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use syn::parse_quote;

    use super::*;

    fn expanded(input: &DeriveInput) -> String {
        expand(input).unwrap().to_string()
    }

    fn error(input: &DeriveInput) -> String {
        expand(input).unwrap_err().to_string()
    }

    #[test]
    fn describes_and_reads_each_kind_of_scheme() {
        let cases: [(DeriveInput, &str, &str); 6] = [
            (
                parse_quote!(
                    #[security(api_key, header = "x-key")]
                    struct Key(String);
                ),
                r#"api_key (:: thermite :: ApiKeyLocation :: Header , "x-key")"#,
                r#"read_api_key (request , :: thermite :: ApiKeyLocation :: Header , "x-key")"#,
            ),
            (
                parse_quote!(
                    #[security(api_key, query = "key")]
                    struct Key(String);
                ),
                "ApiKeyLocation :: Query",
                "read_api_key",
            ),
            (
                parse_quote!(
                    #[security(api_key, cookie = "session")]
                    struct Key(String);
                ),
                "ApiKeyLocation :: Cookie",
                "read_api_key",
            ),
            (
                parse_quote!(
                    #[security(http_basic)]
                    struct Admin(Credentials);
                ),
                "http_basic ()",
                "read_basic_credentials (request) . map (| found | found . map",
            ),
            (
                parse_quote!(
                    #[security(http_bearer)]
                    struct Bearer(Credentials);
                ),
                "http_bearer ()",
                "read_bearer_credentials (request)",
            ),
            (
                parse_quote!(
                    #[security(oauth2_password, token_url = "token")]
                    struct Token(String);
                ),
                r#"oauth2_password ("token")"#,
                "read_bearer_token (request)",
            ),
        ];

        for (input, scheme, reader) in cases {
            let expanded = expanded(&input);
            assert!(expanded.contains(scheme), "{expanded}");
            assert!(expanded.contains(reader), "{expanded}");
            assert!(
                expanded.contains("type Rejection = :: thermite :: NotAuthenticated"),
                "{expanded}"
            );
        }
    }

    #[test]
    fn renames_and_describes_the_scheme_across_attributes() {
        let input = parse_quote! {
            #[security(http_basic, scheme_name = "Admin")]
            #[security(description = "For the admin pages")]
            struct Admin(Credentials);
        };

        assert!(expanded(&input).contains(
            r#"http_basic () . with_scheme_name ("Admin") . with_description ("For the admin pages")"#
        ));
    }

    #[test]
    fn rejects_anything_but_a_newtype() {
        let inputs: [DeriveInput; 4] = [
            parse_quote!(
                #[security(http_basic)]
                struct Pair(String, String);
            ),
            parse_quote!(
                #[security(http_basic)]
                struct Named {
                    token: String,
                }
            ),
            parse_quote!(
                #[security(http_basic)]
                struct Unit;
            ),
            parse_quote!(
                #[security(http_basic)]
                enum Choice {
                    Token(String),
                }
            ),
        ];

        for input in inputs {
            assert_eq!(error(&input), NEWTYPE_ONLY);
        }
    }

    #[test]
    fn rejects_a_generic_struct() {
        let input = parse_quote!(
            #[security(http_basic)]
            struct Admin<T>(T);
        );

        assert_eq!(
            error(&input),
            "`SecurityScheme` cannot be derived for a generic struct"
        );
    }

    #[test]
    fn names_what_is_missing() {
        let cases: [(DeriveInput, String); 3] = [
            (
                parse_quote!(
                    #[security(description = "x")]
                    struct Token(String);
                ),
                format!("add `#[security(...)]` naming a scheme: {SCHEMES}"),
            ),
            (
                parse_quote!(
                    #[security(api_key)]
                    struct Key(String);
                ),
                "`api_key` needs `header`, `query` or `cookie` with the key's name".to_owned(),
            ),
            (
                parse_quote!(
                    #[security(oauth2_password)]
                    struct Token(String);
                ),
                "`oauth2_password` needs `token_url`".to_owned(),
            ),
        ];

        for (input, message) in cases {
            assert_eq!(error(&input), message);
        }
    }

    #[test]
    fn rejects_settings_that_do_not_fit() {
        let cases: [(DeriveInput, String); 9] = [
            (
                parse_quote!(
                    #[security(http_basic, header = "x-key")]
                    struct Key(String);
                ),
                "`header` is only for `api_key`".to_owned(),
            ),
            (
                parse_quote!(
                    #[security(http_bearer, token_url = "token")]
                    struct Token(String);
                ),
                "`token_url` is only for `oauth2_password`".to_owned(),
            ),
            (
                parse_quote!(
                    #[security(http_basic, http_bearer)]
                    struct Token(String);
                ),
                "the scheme is already `http_basic`".to_owned(),
            ),
            (
                parse_quote!(
                    #[security(http_basic = "x")]
                    struct Token(String);
                ),
                "`http_basic` takes no value".to_owned(),
            ),
            (
                parse_quote!(
                    #[security(api_key, header = "a", query = "b")]
                    struct Key(String);
                ),
                "the key is already read from the header".to_owned(),
            ),
            (
                parse_quote!(
                    #[security(http_basic, scheme_name = "a", scheme_name = "b")]
                    struct Admin(String);
                ),
                "`scheme_name` is already set".to_owned(),
            ),
            (
                parse_quote!(
                    #[security(http_basic, realm = "a")]
                    struct Admin(String);
                ),
                unknown_setting(),
            ),
            (
                parse_quote!(
                    #[security(thermite::http_basic)]
                    struct Admin(String);
                ),
                unknown_setting(),
            ),
            (
                parse_quote!(
                    #[security(http_basic, description = 1)]
                    struct Admin(String);
                ),
                "expected string literal".to_owned(),
            ),
        ];

        for (input, message) in cases {
            assert_eq!(error(&input), message);
        }
    }
}
