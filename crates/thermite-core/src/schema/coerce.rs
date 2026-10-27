use serde_json::{Map, Number, Value};

use super::{Field, Kind, Presence, Schema};
use crate::validation::{ErrorKind, Location, ValidationError, collect_all};

pub(crate) type Validated = Result<Value, Vec<ValidationError>>;

enum Coerced {
    Unchanged,
    Converted(Value),
}

// pydantic's lax mode: strings from a URL or JSON become numbers and booleans where they parse
impl Schema {
    pub(crate) fn validate(&self, value: Value, location: &Location<'_>) -> Validated {
        let coerced = match &self.0 {
            Kind::String => string(&value),
            Kind::Integer => integer(&value),
            Kind::Number => number(&value),
            Kind::Boolean => boolean(&value),
            Kind::Array(items) => return array(items, value, location),
            Kind::Object(fields) => return object(fields, value, location),
        };
        match coerced {
            Ok(Coerced::Unchanged) => Ok(value),
            Ok(Coerced::Converted(converted)) => Ok(converted),
            Err(kind) => Err(failure(kind, location, value)),
        }
    }
}

impl Field {
    // `container` is what a missing-field error reports as its input, as pydantic reports the whole object
    pub(crate) fn validate(
        &self,
        value: Option<Value>,
        location: &Location<'_>,
        container: Option<&Value>,
    ) -> Validated {
        match (value, &self.presence) {
            (None, Presence::Required) => Err(failure(
                ErrorKind::Missing,
                location,
                container.cloned().unwrap_or_default(),
            )),
            (None | Some(Value::Null), Presence::Optional) => Ok(Value::Null),
            (None, Presence::Default(default)) => Ok(default.clone()),
            (Some(value), _) => self.schema.validate(value, location),
        }
    }
}

fn failure(kind: ErrorKind, location: &Location<'_>, input: Value) -> Vec<ValidationError> {
    vec![ValidationError::new(kind, location, input)]
}

fn string(value: &Value) -> Result<Coerced, ErrorKind> {
    match value {
        Value::String(_) => Ok(Coerced::Unchanged),
        _ => Err(ErrorKind::StringType),
    }
}

fn integer(value: &Value) -> Result<Coerced, ErrorKind> {
    match value {
        Value::Number(number) if number.is_i64() || number.is_u64() => Ok(Coerced::Unchanged),
        Value::Number(number) => match number.as_f64() {
            Some(float) if float.fract() == 0.0 => {
                integer_from_text(&format!("{float:.0}")).map(Coerced::Converted)
            }
            _ => Err(ErrorKind::IntFromFloat),
        },
        Value::String(text) => integer_from_text(text.trim()).map(Coerced::Converted),
        _ => Err(ErrorKind::IntType),
    }
}

fn integer_from_text(text: &str) -> Result<Value, ErrorKind> {
    // like pydantic, `4.00` is the integer 4, while `4.` and `.0` are not integers
    let text = match text.split_once('.') {
        Some((whole, zeros)) if !zeros.is_empty() && zeros.bytes().all(|byte| byte == b'0') => {
            whole
        }
        _ => text,
    };
    if let Ok(integer) = text.parse::<i64>() {
        return Ok(integer.into());
    }
    if let Ok(integer) = text.parse::<u64>() {
        return Ok(integer.into());
    }
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
        Err(ErrorKind::IntParsingSize)
    } else {
        Err(ErrorKind::IntParsing)
    }
}

fn number(value: &Value) -> Result<Coerced, ErrorKind> {
    let float = match value {
        Value::Number(number) => number.as_f64().ok_or(ErrorKind::FloatType)?,
        Value::String(text) => text
            .trim()
            .parse::<f64>()
            .map_err(|_| ErrorKind::FloatParsing)?,
        _ => return Err(ErrorKind::FloatType),
    };
    // JSON has no NaN or infinity, so serde_json refuses them
    Number::from_f64(float)
        .map(|number| Coerced::Converted(Value::Number(number)))
        .ok_or(ErrorKind::FiniteNumber)
}

fn boolean(value: &Value) -> Result<Coerced, ErrorKind> {
    let converted = match value {
        Value::Bool(_) => return Ok(Coerced::Unchanged),
        Value::Number(number) => match number.as_i64() {
            Some(0) => false,
            Some(1) => true,
            _ => return Err(ErrorKind::BoolParsing),
        },
        Value::String(text) => match text.to_ascii_lowercase().as_str() {
            "1" | "on" | "t" | "true" | "y" | "yes" => true,
            "0" | "off" | "f" | "false" | "n" | "no" => false,
            _ => return Err(ErrorKind::BoolParsing),
        },
        _ => return Err(ErrorKind::BoolType),
    };
    Ok(Coerced::Converted(Value::Bool(converted)))
}

fn array(items: &Schema, value: Value, location: &Location<'_>) -> Validated {
    let Value::Array(elements) = value else {
        return Err(failure(ErrorKind::ListType, location, value));
    };
    collect_all(
        elements
            .into_iter()
            .enumerate()
            .map(|(index, element)| items.validate(element, &location.index(index))),
    )
    .map(Value::Array)
}

fn object(fields: &[Field], value: Value, location: &Location<'_>) -> Validated {
    let Value::Object(mut members) = value else {
        return Err(failure(ErrorKind::ModelAttributesType, location, value));
    };
    // fields are moved out below, so keep a copy for the error input only when one is missing
    let original = fields
        .iter()
        .any(|field| field.is_required() && !members.contains_key(&field.name))
        .then(|| Value::Object(members.clone()));
    collect_all(fields.iter().map(|field| {
        let (name, member) = match members.remove_entry(&field.name) {
            Some((name, member)) => (name, Some(member)),
            None => (field.name.clone(), None),
        };
        field
            .validate(member, &location.key(&field.name), original.as_ref())
            .map(|validated| (name, validated))
    }))
    .map(|entries| Value::Object(entries.into_iter().collect::<Map<_, _>>()))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

    fn check(schema: &Schema, value: Value) -> Validated {
        schema.validate(value, &Location::root("body"))
    }

    fn failed(kind: ErrorKind, location: Value, input: Value) -> Validated {
        Err(vec![ValidationError::at(kind, location, input)])
    }

    #[test]
    fn accepts_only_strings_as_strings() {
        assert_eq!(check(&Schema::string(), json!(" hot ")), Ok(json!(" hot ")));
        assert_eq!(
            check(&Schema::string(), json!(5)),
            failed(ErrorKind::StringType, json!(["body"]), json!(5))
        );
    }

    #[test]
    fn coerces_integers_from_whole_numbers_and_numeric_strings() {
        let integer = Schema::integer();
        let cases = [
            (json!(-5), json!(-5)),
            (json!(u64::MAX), json!(u64::MAX)),
            (json!(u64::MAX.to_string()), json!(u64::MAX)),
            (json!(4.0), json!(4)),
            (json!(" +42 "), json!(42)),
            (json!("4.000"), json!(4)),
            (json!("-0.0"), json!(0)),
        ];

        for (input, expected) in cases {
            assert_eq!(check(&integer, input.clone()), Ok(expected), "{input}");
        }
    }

    #[test]
    fn rejects_integers_that_do_not_parse_or_fit() {
        let integer = Schema::integer();
        let cases = [
            (json!(4.5), ErrorKind::IntFromFloat),
            (json!(1e300), ErrorKind::IntParsingSize),
            (json!("4.5"), ErrorKind::IntParsing),
            (json!("4.01"), ErrorKind::IntParsing),
            (json!("4."), ErrorKind::IntParsing),
            (json!(".0"), ErrorKind::IntParsing),
            (json!("4.0.0"), ErrorKind::IntParsing),
            (json!(""), ErrorKind::IntParsing),
            (json!("99999999999999999999"), ErrorKind::IntParsingSize),
            (json!(true), ErrorKind::IntType),
            (json!(null), ErrorKind::IntType),
        ];

        for (input, kind) in cases {
            assert_eq!(
                check(&integer, input.clone()),
                failed(kind, json!(["body"]), input.clone()),
                "{input}"
            );
        }
    }

    #[test]
    fn coerces_numbers_to_floats() {
        let number = Schema::number();

        assert_eq!(check(&number, json!(2)), Ok(json!(2.0)));
        assert_eq!(check(&number, json!(" 2.5e1")), Ok(json!(25.0)));
        assert_eq!(
            check(&number, json!("2,5")),
            failed(ErrorKind::FloatParsing, json!(["body"]), json!("2,5"))
        );
        assert_eq!(
            check(&number, json!("inf")),
            failed(ErrorKind::FiniteNumber, json!(["body"]), json!("inf"))
        );
        assert_eq!(
            check(&number, json!([])),
            failed(ErrorKind::FloatType, json!(["body"]), json!([]))
        );
    }

    #[test]
    fn coerces_booleans_from_pydantic_words_and_zero_or_one() {
        let boolean = Schema::boolean();
        for word in ["1", "on", "T", "true", "y", "Yes"] {
            assert_eq!(check(&boolean, json!(word)), Ok(json!(true)), "{word}");
        }
        for word in ["0", "OFF", "f", "False", "n", "no"] {
            assert_eq!(check(&boolean, json!(word)), Ok(json!(false)), "{word}");
        }

        assert_eq!(check(&boolean, json!(1)), Ok(json!(true)));
        assert_eq!(check(&boolean, json!(0)), Ok(json!(false)));
        assert_eq!(check(&boolean, json!(false)), Ok(json!(false)));
        assert_eq!(
            check(&boolean, json!(2)),
            failed(ErrorKind::BoolParsing, json!(["body"]), json!(2))
        );
        assert_eq!(
            check(&boolean, json!("maybe")),
            failed(ErrorKind::BoolParsing, json!(["body"]), json!("maybe"))
        );
        assert_eq!(
            check(&boolean, json!({})),
            failed(ErrorKind::BoolType, json!(["body"]), json!({}))
        );
    }

    #[test]
    fn validates_every_array_element_and_reports_each_index() {
        let numbers = Schema::array(Schema::integer());

        assert_eq!(check(&numbers, json!(["1", 2])), Ok(json!([1, 2])));
        assert_eq!(
            check(&numbers, json!(["a", 2, "b"])),
            Err(vec![
                ValidationError::at(ErrorKind::IntParsing, json!(["body", 0]), json!("a")),
                ValidationError::at(ErrorKind::IntParsing, json!(["body", 2]), json!("b")),
            ])
        );
        assert_eq!(
            check(&numbers, json!("1")),
            failed(ErrorKind::ListType, json!(["body"]), json!("1"))
        );
    }

    #[test]
    fn validates_object_fields_in_schema_order_and_drops_unknown_ones() {
        let item = Schema::object([
            Field::new("name", Schema::string()),
            Field::new("price", Schema::number()),
        ]);

        let validated = check(&item, json!({"price": 3, "extra": true, "name": "anvil"}));

        assert_eq!(
            validated.map(|value| value.to_string()),
            Ok(r#"{"name":"anvil","price":3.0}"#.to_owned())
        );
    }

    #[test]
    fn reports_a_missing_field_with_the_whole_object_as_input() {
        let item = Schema::object([
            Field::new("name", Schema::string()),
            Field::new("price", Schema::number()),
        ]);

        assert_eq!(
            check(&item, json!({"price": "free"})),
            Err(vec![
                ValidationError::at(
                    ErrorKind::Missing,
                    json!(["body", "name"]),
                    json!({"price": "free"})
                ),
                ValidationError::at(
                    ErrorKind::FloatParsing,
                    json!(["body", "price"]),
                    json!("free")
                ),
            ])
        );
        assert_eq!(
            check(&item, json!([])),
            failed(ErrorKind::ModelAttributesType, json!(["body"]), json!([]))
        );
    }

    #[test]
    fn fills_in_optional_and_default_fields() {
        let item = Schema::object([
            Field::new("description", Schema::string()).optional(),
            Field::new("tax", Schema::number()).optional(),
            Field::new("count", Schema::integer()).with_default(1),
        ]);

        assert_eq!(
            check(&item, json!({"tax": null})),
            Ok(json!({"description": null, "tax": null, "count": 1}))
        );
        assert_eq!(
            check(&item, json!({"count": null})),
            failed(ErrorKind::IntType, json!(["body", "count"]), json!(null))
        );
    }

    #[test]
    fn locates_errors_in_nested_values() {
        let order = Schema::object([Field::new(
            "items",
            Schema::array(Schema::object([Field::new("id", Schema::integer())])),
        )]);

        assert_eq!(
            check(&order, json!({"items": [{"id": 1}, {"id": "x"}]})),
            failed(
                ErrorKind::IntParsing,
                json!(["body", "items", 1, "id"]),
                json!("x")
            )
        );
    }

    proptest! {
        #[test]
        fn any_i64_written_as_text_becomes_that_integer(integer in any::<i64>()) {
            prop_assert_eq!(check(&Schema::integer(), json!(integer.to_string())), Ok(json!(integer)));
        }

        #[test]
        fn any_finite_float_written_as_text_becomes_that_number(float in any::<f64>().prop_filter("finite", |float| float.is_finite())) {
            prop_assert_eq!(check(&Schema::number(), json!(float.to_string())), Ok(json!(float)));
        }

        #[test]
        fn validating_any_string_returns_without_panicking(text in ".*") {
            for schema in [Schema::integer(), Schema::number(), Schema::boolean()] {
                let _ = check(&schema, json!(text));
            }
        }
    }
}
