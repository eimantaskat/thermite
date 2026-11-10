use serde::{Deserialize, Serialize};
use thermite::{
    App, Field, Json, Model, ParameterLocation, Query, RouteError, Schema, SchemaLocation,
    StatusCode, ToSchema,
};

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
struct Plain {
    #[model(default)]
    name: String,
    #[model(default)]
    count: i64,
    #[model(default)]
    ratio: f64,
    #[model(default)]
    done: bool,
    #[model(default)]
    tags: Vec<Tag>,
}

#[test]
fn a_plain_default_is_the_empty_value_of_the_type() {
    let tag = Schema::model("Tag", [Field::new("label", Schema::string())]);
    assert_eq!(
        Plain::fields(),
        [
            Field::new("name", Schema::string()).with_default(""),
            Field::new("count", Schema::integer()).with_default(0),
            Field::new("ratio", Schema::number()).with_default(0.0),
            Field::new("done", Schema::boolean()).with_default(false),
            Field::new("tags", Schema::array(tag)).with_default(Vec::<String>::new()),
        ]
    );
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
            location: SchemaLocation::Parameter(ParameterLocation::Query),
            name: "limit".to_owned()
        })
    );
}

#[derive(Deserialize, Model)]
#[allow(dead_code, reason = "only the derived fields and the replies are read")]
struct Comment {
    text: String,
    replies: Vec<Comment>,
}

impl Comment {
    fn count(&self) -> usize {
        1 + self.replies.iter().map(Comment::count).sum::<usize>()
    }
}

#[derive(Deserialize, Model)]
#[allow(dead_code, reason = "only the derived fields are read")]
struct Author {
    name: String,
    posts: Vec<Post>,
}

#[derive(Deserialize, Model)]
#[allow(dead_code, reason = "only the derived fields are read")]
struct Post {
    title: String,
    author: Option<Author>,
}

#[test]
fn a_model_inside_itself_is_declared_by_reference() {
    assert_eq!(
        Comment::schema(),
        Schema::model(
            "Comment",
            [
                Field::new("text", Schema::string()),
                Field::new("replies", Schema::array(Schema::reference("Comment"))),
            ]
        )
    );
    assert_eq!(
        Author::schema(),
        Schema::model(
            "Author",
            [
                Field::new("name", Schema::string()),
                Field::new(
                    "posts",
                    Schema::array(Schema::model(
                        "Post",
                        [
                            Field::new("title", Schema::string()),
                            Field::new("author", Schema::reference("Author")).optional(),
                        ]
                    ))
                ),
            ]
        )
    );
}

async fn count_comments(Json(comment): Json<Comment>) -> String {
    comment.count().to_string()
}

#[tokio::test]
async fn a_model_inside_itself_validates_at_every_depth() -> TestResult {
    let server = RunningServer::start(App::new().post("/comments", count_comments)?).await?;
    let client = reqwest::Client::new();
    let thread =
        r#"{"text": "a", "replies": [{"text": "b", "replies": [{"text": "c", "replies": []}]}]}"#;
    let broken =
        r#"{"text": "a", "replies": [{"text": "b", "replies": [{"text": 3, "replies": []}]}]}"#;

    let counted = client
        .post(server.url("/comments"))
        .body(thread)
        .send()
        .await?;
    let rejected = client
        .post(server.url("/comments"))
        .body(broken)
        .send()
        .await?;

    assert_eq!(counted.text().await?, "3");
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        rejected
            .text()
            .await?
            .contains(r#""loc":["body","replies",0,"replies",0,"text"]"#)
    );
    server.stop().await
}

mod first {
    use serde::Deserialize;
    use thermite::Model;

    #[derive(Deserialize, Model)]
    #[allow(dead_code, reason = "only the derived fields are read")]
    pub struct Item {
        inner: super::second::Item,
        id: i64,
    }
}

mod second {
    use serde::Deserialize;
    use thermite::Model;

    #[derive(Deserialize, Model)]
    #[allow(dead_code, reason = "only the derived fields are read")]
    pub struct Item {
        label: String,
        back: Vec<super::first::Item>,
    }
}

// a reference names the model by its short name, so validation could not tell the two apart
#[test]
fn two_models_of_one_name_in_a_body_fail_when_the_route_is_registered() {
    let registered = App::new()
        .without_openapi()
        .post("/items", |_: Json<first::Item>| async {});

    assert_eq!(
        registered.err(),
        Some(RouteError::ModelNameConflict {
            path: "/items".to_owned(),
            location: SchemaLocation::Body,
            name: "Item".to_owned()
        })
    );
}
