use serde::{Deserialize, Serialize};
use thermite::{App, Field, Json, Model, Query, RouteError, Schema, StatusCode};

use crate::support::{RunningServer, TestResult};

#[derive(Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code, reason = "only the derived fields are read")]
struct Search {
    page_size: Option<i64>,
    #[model(default = Vec::<String>::new())]
    tag_names: Vec<String>,
    #[serde(rename = "q")]
    query_text: String,
    r#type: bool,
}

#[test]
fn derives_the_fields_serde_reads() {
    assert_eq!(
        Search::fields(),
        [
            Field::new("pageSize", Schema::integer()).optional(),
            Field::new("tagNames", Schema::array(Schema::string()))
                .with_default(Vec::<String>::new()),
            Field::new("q", Schema::string()),
            Field::new("type", Schema::boolean()),
        ]
    );
}

#[derive(Deserialize, Model)]
#[allow(dead_code, reason = "only the derived fields are read")]
struct Tag {
    label: String,
}

#[derive(Deserialize, Model)]
#[allow(dead_code, reason = "only the derived fields are read")]
struct Page<T> {
    items: Vec<T>,
    #[model(default = 1)]
    number: i64,
}

#[test]
fn a_generic_model_declares_its_parameters_schemas() {
    assert_eq!(Page::<Tag>::name(), "Page_Tag_");
    assert_eq!(
        Page::<Tag>::fields(),
        [
            Field::new(
                "items",
                Schema::array(Schema::model(
                    "Tag",
                    [Field::new("label", Schema::string())]
                ))
            ),
            Field::new("number", Schema::integer()).with_default(1),
        ]
    );
}

#[derive(Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
struct Order {
    item_name: String,
    #[model(default = 1)]
    quantity: i64,
    gift_note: Option<String>,
}

#[derive(Deserialize, Model)]
#[serde(rename_all = "kebab-case")]
struct Options {
    #[model(default = false)]
    dry_run: bool,
}

async fn place_order(Query(options): Query<Options>, Json(order): Json<Order>) -> Json<Order> {
    Json(Order {
        item_name: format!("{} (dry run: {})", order.item_name, options.dry_run),
        ..order
    })
}

#[tokio::test]
async fn a_derived_model_reads_what_serde_deserializes() -> TestResult {
    let server = RunningServer::start(App::new().post("/orders", place_order)?).await?;
    let client = reqwest::Client::new();

    let response = client
        .post(server.url("/orders?dry-run=true"))
        .body(r#"{"itemName": "anvil"}"#)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.text().await?,
        r#"{"itemName":"anvil (dry run: true)","quantity":1,"giftNote":null}"#
    );
    server.stop().await
}

#[derive(Deserialize, Model)]
#[allow(dead_code, reason = "only the derived fields are read")]
struct WrongDefault {
    #[model(default = "ten")]
    limit: i64,
}

#[test]
fn a_default_that_does_not_fit_its_type_fails_when_the_route_is_registered() {
    let registered = App::new().get("/items", |_: Query<WrongDefault>| async {});

    assert_eq!(
        registered.err(),
        Some(RouteError::InvalidDefault {
            path: "/items".to_owned(),
            name: "limit".to_owned()
        })
    );
}
