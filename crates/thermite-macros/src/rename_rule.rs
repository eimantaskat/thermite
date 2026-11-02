use syn::LitStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameRule {
    Lower,
    Upper,
    Pascal,
    Camel,
    Snake,
    ScreamingSnake,
    Kebab,
    ScreamingKebab,
}

const RULES: [(&str, RenameRule); 8] = [
    ("lowercase", RenameRule::Lower),
    ("UPPERCASE", RenameRule::Upper),
    ("PascalCase", RenameRule::Pascal),
    ("camelCase", RenameRule::Camel),
    ("snake_case", RenameRule::Snake),
    ("SCREAMING_SNAKE_CASE", RenameRule::ScreamingSnake),
    ("kebab-case", RenameRule::Kebab),
    ("SCREAMING-KEBAB-CASE", RenameRule::ScreamingKebab),
];

impl RenameRule {
    pub(crate) fn parse(literal: &LitStr) -> syn::Result<Self> {
        let value = literal.value();
        RULES
            .iter()
            .find(|(name, _)| *name == value)
            .map(|(_, rule)| *rule)
            .ok_or_else(|| {
                let names = RULES.map(|(name, _)| format!("\"{name}\"")).join(", ");
                syn::Error::new(
                    literal.span(),
                    format!("unknown rename rule \"{value}\", expected one of {names}"),
                )
            })
    }

    // serde's rules for field names, which start out in snake_case
    pub(crate) fn apply(self, field: &str) -> String {
        match self {
            Self::Lower | Self::Snake => field.to_owned(),
            Self::Upper | Self::ScreamingSnake => field.to_ascii_uppercase(),
            Self::Pascal => pascal_case(field),
            Self::Camel => {
                let pascal = pascal_case(field);
                let mut characters = pascal.chars();
                characters.next().map_or_else(String::new, |first| {
                    first.to_ascii_lowercase().to_string() + characters.as_str()
                })
            }
            Self::Kebab => field.replace('_', "-"),
            Self::ScreamingKebab => field.to_ascii_uppercase().replace('_', "-"),
        }
    }
}

fn pascal_case(field: &str) -> String {
    let mut pascal = String::with_capacity(field.len());
    let mut capitalize = true;
    for character in field.chars() {
        if character == '_' {
            capitalize = true;
        } else if capitalize {
            pascal.push(character.to_ascii_uppercase());
            capitalize = false;
        } else {
            pascal.push(character);
        }
    }
    pascal
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use syn::parse_quote;

    use super::*;

    #[test]
    fn renames_snake_case_fields_like_serde() {
        let renamed = RULES.map(|(name, rule)| (name, rule.apply("user_id_2")));

        assert_eq!(
            renamed,
            [
                ("lowercase", "user_id_2".to_owned()),
                ("UPPERCASE", "USER_ID_2".to_owned()),
                ("PascalCase", "UserId2".to_owned()),
                ("camelCase", "userId2".to_owned()),
                ("snake_case", "user_id_2".to_owned()),
                ("SCREAMING_SNAKE_CASE", "USER_ID_2".to_owned()),
                ("kebab-case", "user-id-2".to_owned()),
                ("SCREAMING-KEBAB-CASE", "USER-ID-2".to_owned()),
            ]
        );
    }

    #[test]
    fn camel_case_of_only_underscores_is_empty() {
        assert_eq!(RenameRule::Camel.apply("_"), "");
    }

    #[test]
    fn parses_serde_rule_names() {
        for (name, rule) in RULES {
            assert_eq!(
                RenameRule::parse(&LitStr::new(name, proc_macro2::Span::call_site())).unwrap(),
                rule
            );
        }
    }

    #[test]
    fn lists_the_rules_when_the_name_is_unknown() {
        let error = RenameRule::parse(&parse_quote!("Title Case")).unwrap_err();

        assert_eq!(
            error.to_string(),
            "unknown rename rule \"Title Case\", expected one of \"lowercase\", \"UPPERCASE\", \
             \"PascalCase\", \"camelCase\", \"snake_case\", \"SCREAMING_SNAKE_CASE\", \
             \"kebab-case\", \"SCREAMING-KEBAB-CASE\""
        );
    }
}
