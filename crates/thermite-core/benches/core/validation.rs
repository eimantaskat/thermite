use bytes::Bytes;
use criterion::{Criterion, criterion_group};
use http::header::CONTENT_TYPE;
use http::{Method, StatusCode};
use serde_json::{Value, json};
use thermite_core::{Field, RouteDescription, Schema};

use crate::support::{bench_handle, request, router, runtime};

fn item_schema() -> Schema {
    Schema::model(
        "Item",
        [
            Field::new("name", Schema::string()),
            Field::new("price", Schema::number()),
            Field::new("quantity", Schema::integer()),
            Field::new("active", Schema::boolean()),
            Field::new("tags", Schema::array(Schema::string())),
        ],
    )
}

fn order_schema() -> Schema {
    let address = Schema::model(
        "Address",
        [
            Field::new("street", Schema::string()),
            Field::new("city", Schema::string()),
            Field::new("zip", Schema::string()),
        ],
    );
    let customer = Schema::model(
        "Customer",
        [
            Field::new("name", Schema::string()),
            Field::new("email", Schema::string()),
            Field::new("address", address),
        ],
    );
    Schema::model(
        "Order",
        [
            Field::new("id", Schema::integer()),
            Field::new("customer", customer),
            Field::new("items", Schema::array(item_schema())),
        ],
    )
}

fn item(index: usize) -> Value {
    json!({
        "name": format!("item {index}"),
        "price": 9.5,
        "quantity": index,
        "active": true,
        "tags": ["hot", "new"],
    })
}

// every field has the wrong type, so each is one error in the 422
fn invalid_item() -> Value {
    json!({
        "name": ["not", "a", "string"],
        "price": "free",
        "quantity": 1.5,
        "active": "maybe",
        "tags": "none",
    })
}

fn order() -> Value {
    json!({
        "id": 1,
        "customer": {
            "name": "Ada",
            "email": "ada@example.com",
            "address": {"street": "1 Main St", "city": "London", "zip": "N1"},
        },
        "items": (0..3).map(item).collect::<Vec<_>>(),
    })
}

fn items(count: usize) -> Value {
    Value::Array((0..count).map(item).collect())
}

fn body_route(path: &str, schema: Schema) -> RouteDescription {
    RouteDescription::new(Method::POST, path).body(schema)
}

fn validation(criterion: &mut Criterion) {
    let runtime = runtime();
    let router = router([
        body_route("/item", item_schema()),
        body_route("/order", order_schema()),
        body_route("/items", Schema::array(item_schema())),
    ]);
    let cases = [
        ("flat", "/item", item(0), StatusCode::OK),
        ("nested", "/order", order(), StatusCode::OK),
        ("list/100", "/items", items(100), StatusCode::OK),
        ("list/10000", "/items", items(10_000), StatusCode::OK),
        (
            "invalid/flat",
            "/item",
            invalid_item(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "invalid/list/100",
            "/items",
            Value::Array(vec![invalid_item(); 100]),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ];

    let mut group = criterion.benchmark_group("validation");
    for (name, path, body, expected) in cases {
        let body = Bytes::from(body.to_string());
        bench_handle(&mut group, &runtime, name, &router, expected, || {
            request(
                http::Request::post(path).header(CONTENT_TYPE, "application/json"),
                body.clone(),
            )
        });
    }
    bench_handle(
        &mut group,
        &runtime,
        "invalid/json",
        &router,
        StatusCode::UNPROCESSABLE_ENTITY,
        || {
            request(
                http::Request::post("/item").header(CONTENT_TYPE, "application/json"),
                r#"{"name": "anvil", "price": "#,
            )
        },
    );
    group.finish();
}

criterion_group!(benches, validation);
