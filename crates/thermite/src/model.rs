use std::any::type_name;

use thermite_core::{Field, Schema};

pub trait ToSchema {
    fn schema() -> Schema;
}

// a struct whose fields the extractors can declare one by one
pub trait Model {
    fn fields() -> Vec<Field>;

    // the name the docs list the model under
    #[must_use]
    fn name() -> String {
        schema_name(type_name::<Self>())
    }
}

impl<T: Model> ToSchema for T {
    fn schema() -> Schema {
        Schema::model(T::name(), T::fields())
    }
}

// `app::Page<app::Item>` becomes `Page_Item_`, as pydantic names `Page[Item]`, using only characters OpenAPI allows
fn schema_name(type_name: &str) -> String {
    let mut name = String::with_capacity(type_name.len());
    let mut segment_start = 0;
    let mut rest = type_name;
    while let Some(character) = rest.chars().next() {
        if let Some(after_separator) = rest.strip_prefix("::") {
            name.truncate(segment_start);
            rest = after_separator;
            continue;
        }
        if character.is_ascii_alphanumeric() || character == '_' {
            name.push(character);
        } else {
            name.push('_');
            segment_start = name.len();
        }
        rest = &rest[character.len_utf8()..];
    }
    name
}

impl ToSchema for String {
    fn schema() -> Schema {
        Schema::string()
    }
}

// other integer widths wait for range constraints, so a validated value always fits
impl ToSchema for i64 {
    fn schema() -> Schema {
        Schema::integer()
    }
}

impl ToSchema for f64 {
    fn schema() -> Schema {
        Schema::number()
    }
}

impl ToSchema for bool {
    fn schema() -> Schema {
        Schema::boolean()
    }
}

impl<T: ToSchema> ToSchema for Vec<T> {
    fn schema() -> Schema {
        Schema::array(T::schema())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Item;

    impl Model for Item {
        fn fields() -> Vec<Field> {
            vec![
                Field::new("name", String::schema()),
                Field::new("tags", Vec::<String>::schema()).with_default(Vec::<String>::new()),
            ]
        }
    }

    #[test]
    fn maps_rust_types_to_schemas() {
        assert_eq!(String::schema(), Schema::string());
        assert_eq!(i64::schema(), Schema::integer());
        assert_eq!(f64::schema(), Schema::number());
        assert_eq!(bool::schema(), Schema::boolean());
        assert_eq!(
            Vec::<Vec<bool>>::schema(),
            Schema::array(Schema::array(Schema::boolean()))
        );
    }

    struct Page<T>(T);

    impl<T> Model for Page<T> {
        fn fields() -> Vec<Field> {
            Vec::new()
        }
    }

    // type_name's format may change between compilers, so this pins it on each toolchain bump
    #[test]
    fn a_model_is_named_after_its_type_without_module_paths() {
        assert_eq!(Item::name(), "Item");
        assert_eq!(Page::<Item>::name(), "Page_Item_");
        assert_eq!(Page::<(i64, &str)>::name(), "Page__i64___str__");
    }

    #[test]
    fn a_model_is_an_object_of_its_fields() {
        assert_eq!(
            Vec::<Item>::schema(),
            Schema::array(Schema::model(
                "Item",
                [
                    Field::new("name", Schema::string()),
                    Field::new("tags", Schema::array(Schema::string()))
                        .with_default(Vec::<String>::new()),
                ]
            ))
        );
    }
}
