use thermite_core::{Field, Schema};

pub trait ToSchema {
    fn schema() -> Schema;
}

// a struct whose fields the extractors can declare one by one
pub trait Model {
    fn fields() -> Vec<Field>;
}

impl<T: Model> ToSchema for T {
    fn schema() -> Schema {
        Schema::object(T::fields())
    }
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

    #[test]
    fn a_model_is_an_object_of_its_fields() {
        assert_eq!(
            Vec::<Item>::schema(),
            Schema::array(Schema::object([
                Field::new("name", Schema::string()),
                Field::new("tags", Schema::array(Schema::string()))
                    .with_default(Vec::<String>::new()),
            ]))
        );
    }
}
