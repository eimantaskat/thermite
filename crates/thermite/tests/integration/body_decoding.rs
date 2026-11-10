use std::future::{Future, ready};

use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;
use proptest::test_runner::{Config, TestRunner};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thermite::http::header::CONTENT_TYPE;
use thermite::{
    App, Arguments, Bytes, FromRequest, Json, Model, Request, Response, RouteDescription,
    StatusCode, TestClient, ToSchema,
};
use tokio::runtime::Builder;

use crate::support::TestResult;

#[derive(Serialize, Deserialize, Model)]
struct Owner {
    name: String,
}

#[derive(Serialize, Deserialize, Model)]
struct Item {
    name: String,
    price: f64,
    quantity: i64,
    active: bool,
    tags: Vec<String>,
    note: Option<String>,
    #[model(default = 1)]
    count: i64,
    owner: Owner,
    extra: Value,
}

// the body as the core validated it, the only path `Json` had before it decoded bodies itself
struct Validated<T>(T);

impl<T: ToSchema + DeserializeOwned + Send> FromRequest for Validated<T> {
    type Rejection = Response;

    fn describe(description: RouteDescription) -> RouteDescription {
        description.body(T::schema())
    }

    fn from_request(
        _: &Request,
        arguments: &Arguments,
    ) -> impl Future<Output = Result<Self, Response>> + Send {
        let body = arguments.body().unwrap_or(&Value::Null);
        ready(T::deserialize(body).map(Self).map_err(|_| {
            let mut response = Response::default();
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response
        }))
    }
}

fn app() -> Result<App, thermite::RouteError> {
    App::new()
        .post("/decoded", |Json(items): Json<Vec<Item>>| async move {
            Json(items)
        })?
        .post(
            "/validated",
            |Validated(items): Validated<Vec<Item>>| async move { Json(items) },
        )
}

// a field picks one of its values or, when it may, is left out
fn field(values: &[Value], may_be_missing: bool) -> BoxedStrategy<Option<Value>> {
    let value = proptest::sample::select(values.to_vec()).prop_map(Some);
    if may_be_missing {
        prop_oneof![4 => value, 1 => Just(None)].boxed()
    } else {
        value.boxed()
    }
}

// a clean item is valid as it is, so serde decodes it; any other mixes in values to coerce, invalid ones and gaps
fn item(clean: bool) -> impl Strategy<Value = Value> {
    let pick = |valid: Vec<Value>, others: Vec<Value>, optional: bool| {
        let values = if clean {
            valid
        } else {
            [valid, others].concat()
        };
        field(&values, optional || !clean)
    };
    let fields = (
        pick(
            vec![json!("anvil"), json!("")],
            vec![json!(5), json!(null), json!(["a"])],
            false,
        ),
        pick(
            vec![json!(9.5), json!(3)],
            vec![json!("2.5"), json!("x"), json!(true), json!(null)],
            false,
        ),
        pick(
            vec![json!(4), json!(-1)],
            vec![
                json!(4.0),
                json!(4.5),
                json!("7"),
                json!("x"),
                json!(9_223_372_036_854_775_808_u64),
            ],
            false,
        ),
        pick(
            vec![json!(true), json!(false)],
            vec![json!("true"), json!(1), json!("yes"), json!(2)],
            false,
        ),
        pick(
            vec![json!(["a", "b"]), json!([])],
            vec![json!("a"), json!([1]), json!(null)],
            false,
        ),
        pick(vec![json!("note"), json!(null)], vec![json!(3)], true),
        pick(
            vec![json!(2)],
            vec![json!("3"), json!(null), json!(1.0)],
            true,
        ),
        pick(
            vec![json!({"name": "ada"}), json!({"name": "ada", "age": 36})],
            vec![json!({}), json!("ada"), json!(null)],
            false,
        ),
        pick(
            vec![json!(null), json!({"a": [1]}), json!("text"), json!(1.5)],
            Vec::new(),
            false,
        ),
        pick(vec![json!("unknown"), json!(null)], Vec::new(), true),
    );
    fields.prop_map(
        |(name, price, quantity, active, tags, note, count, owner, extra, unknown)| {
            let names = [
                "name", "price", "quantity", "active", "tags", "note", "count", "owner", "extra",
                "unknown",
            ];
            let values = [
                name, price, quantity, active, tags, note, count, owner, extra, unknown,
            ];
            let object: Map<String, Value> = names
                .into_iter()
                .zip(values)
                .filter_map(|(name, value)| Some((name.to_owned(), value?)))
                .collect();
            Value::Object(object)
        },
    )
}

fn body() -> impl Strategy<Value = Value> {
    prop_oneof![
        4 => proptest::collection::vec(item(true), 1..4).prop_map(Value::Array),
        4 => proptest::collection::vec(item(false), 0..4).prop_map(Value::Array),
        1 => item(true),
        1 => Just(json!(null)),
    ]
}

#[test]
fn a_decoded_body_answers_as_the_validated_one_does() -> TestResult {
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let client = runtime.block_on(TestClient::start(app()?))?;
    let send = |path: &'static str, body: Bytes| {
        runtime.block_on(async {
            let response = client
                .post(path)
                .header(CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await?;
            Ok::<_, thermite::TestRequestError>((response.status(), response.body().clone()))
        })
    };

    let mut runner = TestRunner::new(Config::with_cases(512));
    runner
        .run(&body(), |body| {
            let body = Bytes::from(body.to_string());
            let decoded = send("/decoded", body.clone())
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let validated =
                send("/validated", body).map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_ne!(validated.0, StatusCode::INTERNAL_SERVER_ERROR);
            prop_assert_eq!(decoded, validated);
            Ok(())
        })
        .map_err(|error| error.to_string())?;

    // raw bodies serde_json cannot write: a repeated key, which pydantic and the core read as the last one
    for body in [
        r#"[{"name": "a", "name": "b", "price": 1, "quantity": 1, "active": true, "tags": [], "owner": {"name": "o"}, "extra": 1}]"#,
        r#"[{"name": "a", "price": 1, "quantity": 1, "active": true, "tags": [], "owner": {"name": "o", "name": 2}, "extra": 1}]"#,
        r#"[{"name": "a""#,
    ] {
        let decoded = send("/decoded", Bytes::from_static(body.as_bytes()))?;
        let validated = send("/validated", Bytes::from_static(body.as_bytes()))?;
        assert_eq!(decoded, validated, "{body}");
    }
    runtime.block_on(client.stop());
    Ok(())
}
