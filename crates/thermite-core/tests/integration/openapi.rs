use std::error::Error;

use bytes::Bytes;
use http::Method;
use serde_json::{Value, json};
use thermite_core::{
    Arguments, Docs, Field, OpenApi, Parameter, Request, Response, RouteDescription, Router, Schema,
};

use crate::support::one_route;

fn get(path: &str) -> Result<Request, http::Error> {
    http::Request::builder().uri(path).body(Bytes::new())
}

fn router() -> Result<Router, Box<dyn Error>> {
    Ok(one_route(
        RouteDescription::new(Method::PUT, "/items/{id}")
            .with_name("replace_item")
            .with_tag("items")
            .with_tag("admin")
            .parameter(Parameter::path("id", Schema::integer()))
            .body(Schema::model(
                "Item",
                [Field::new("name", Schema::string())],
            )),
        |_: Request, _: Arguments| async { Response::new(Bytes::new()) },
    )?)
}

#[test]
fn documents_a_route_description_built_by_hand() -> Result<(), Box<dyn Error>> {
    let docs = Docs::new(&OpenApi::new().title("Items"), &router()?)?;

    let response = docs.respond(&get("/openapi.json")?).ok_or("no document")?;
    let document: Value = serde_json::from_slice(response.body())?;

    assert_eq!(
        document["info"],
        json!({"title": "Items", "version": "0.1.0"})
    );
    let operation = &document["paths"]["/items/{id}"]["put"];
    let keys: Vec<&String> = operation
        .as_object()
        .ok_or("no operation")?
        .keys()
        .collect();
    assert_eq!(
        keys,
        [
            "tags",
            "summary",
            "operationId",
            "parameters",
            "requestBody",
            "responses"
        ]
    );
    assert_eq!(operation["tags"], json!(["items", "admin"]));
    assert_eq!(operation["summary"], "Replace Item");
    assert_eq!(
        document["paths"]["/items/{id}"]["put"]["operationId"],
        "replace_item_items__id__put"
    );
    assert_eq!(document["components"]["schemas"]["Item"]["title"], "Item");
    assert!(docs.respond(&get("/items/1")?).is_none());
    Ok(())
}
