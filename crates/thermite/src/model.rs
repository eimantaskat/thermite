use std::any::type_name;
use std::cell::RefCell;

use thermite_core::{Field, Schema, Value};

#[diagnostic::on_unimplemented(
    message = "`{Self}` has no schema, so it cannot be validated or documented",
    label = "no schema for this type",
    note = "derive `Model` on a struct, or implement `ToSchema` for the type"
)]
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
        match Describing::start::<T>() {
            Some(_describing) => Schema::model(T::name(), T::fields()),
            None => Schema::reference(T::name()),
        }
    }
}

thread_local! {
    static DESCRIBING: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

// the models whose fields are being built on this thread, so that one met again inside itself becomes a reference
struct Describing;

impl Describing {
    // keyed by the full type name, which tells apart two models that share a short name
    fn start<T: ?Sized>() -> Option<Self> {
        DESCRIBING.with_borrow_mut(|describing| {
            let key = type_name::<T>();
            if describing.contains(&key) {
                None
            } else {
                describing.push(key);
                Some(Self)
            }
        })
    }
}

// dropped even when `fields` panics, so a caught panic leaves no model marked on the thread
impl Drop for Describing {
    fn drop(&mut self) {
        DESCRIBING.with_borrow_mut(Vec::pop);
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

impl ToSchema for Value {
    fn schema() -> Schema {
        Schema::any()
    }
}

impl<T: ToSchema> ToSchema for Vec<T> {
    fn schema() -> Schema {
        Schema::array(T::schema())
    }
}

#[cfg(test)]
mod tests {
    use std::panic;
    use std::sync::atomic::{AtomicBool, Ordering};

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
        assert_eq!(Value::schema(), Schema::any());
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

    mod outer {
        use super::*;

        pub(super) struct Item;

        impl Model for Item {
            fn fields() -> Vec<Field> {
                vec![Field::new("inner", super::inner::Item::schema())]
            }
        }
    }

    mod inner {
        use super::*;

        pub(super) struct Item;

        impl Model for Item {
            fn fields() -> Vec<Field> {
                vec![Field::new("label", String::schema())]
            }
        }
    }

    // registering a route then reports the two names as a conflict, instead of validation reading one as the other
    #[test]
    fn a_model_inside_another_of_the_same_name_is_not_a_reference() {
        assert_eq!(
            outer::Item::schema(),
            Schema::model(
                "Item",
                [Field::new(
                    "inner",
                    Schema::model("Item", [Field::new("label", Schema::string())])
                )]
            )
        );
    }

    static FAILED_ONCE: AtomicBool = AtomicBool::new(false);

    struct FailsOnce;

    impl Model for FailsOnce {
        fn fields() -> Vec<Field> {
            assert!(FAILED_ONCE.swap(true, Ordering::SeqCst), "first call");
            Vec::new()
        }
    }

    #[test]
    fn a_panic_while_building_fields_leaves_the_model_unmarked() {
        let failed = panic::catch_unwind(FailsOnce::schema);

        assert!(failed.is_err());
        assert_eq!(FailsOnce::schema(), Schema::model("FailsOnce", []));
    }
}
