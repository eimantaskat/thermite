// the functions show types on the page, so only its tests call them
#![cfg_attr(not(test), expect(dead_code))]

// ANCHOR: imports
use std::error::Error;

use serde::{Deserialize, Serialize};
use thermite::{App, Json, Model, RouteError, Server, post, routes};
// ANCHOR_END: imports

// ANCHOR: full_name
fn get_full_name(first_name: &str, last_name: &str) -> String {
    format!("{first_name} {last_name}")
}
// ANCHOR_END: full_name

// ANCHOR: name_with_age
fn get_name_with_age(name: &str, age: i64) -> String {
    format!("{name} is this old: {age}")
}
// ANCHOR_END: name_with_age

// ANCHOR: generics
fn shout_items(items: &[String]) -> Vec<String> {
    items.iter().map(|item| item.to_uppercase()).collect()
}

fn swap(pair: (i64, String)) -> (String, i64) {
    let (number, text) = pair;
    (text, number)
}
// ANCHOR_END: generics

// ANCHOR: option
fn say_hi(name: Option<&str>) -> String {
    match name {
        Some(name) => format!("Hey {name}!"),
        None => "Hello World".to_owned(),
    }
}
// ANCHOR_END: option

// ANCHOR: person
struct Person {
    name: String,
}

impl Person {
    fn greeting(&self) -> String {
        format!("Hello, {}", self.name)
    }
}
// ANCHOR_END: person

// ANCHOR: model
#[derive(Deserialize, Serialize, Model)]
struct User {
    id: i64,
    #[model(default = "John Doe")]
    name: String,
    signup_ts: Option<String>,
    #[model(default)]
    friends: Vec<i64>,
}

#[post("/users")]
async fn create_user(Json(user): Json<User>) -> Json<User> {
    Json(user)
}
// ANCHOR_END: model

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![create_user])
}
// ANCHOR_END: app

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = Server::bind("127.0.0.1:8000").await?;
    server.serve(app()?).await?;
    Ok(())
}

// ANCHOR: test
#[cfg(test)]
mod tests {
    use thermite::TestClient;

    use super::*;

    #[test]
    fn the_functions_answer_as_their_types_say() {
        let person = Person {
            name: "Ferris".to_owned(),
        };

        assert_eq!(get_full_name("john", "doe"), "john doe");
        assert_eq!(get_name_with_age("John", 42), "John is this old: 42");
        assert_eq!(shout_items(&["a".to_owned(), "b".to_owned()]), ["A", "B"]);
        assert_eq!(swap((1, "one".to_owned())), ("one".to_owned(), 1));
        assert_eq!(say_hi(Some("Ferris")), "Hey Ferris!");
        assert_eq!(say_hi(None), "Hello World");
        assert_eq!(person.greeting(), "Hello, Ferris");
    }

    #[tokio::test]
    async fn a_user_is_read_into_its_types() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .post("/users")
            .body(r#"{"id": "123", "signup_ts": "2017-06-01 12:22", "friends": [1, "2", 3]}"#)
            .header("content-type", "application/json")
            .send()
            .await?;

        assert_eq!(
            response.text()?,
            r#"{"id":123,"name":"John Doe","signup_ts":"2017-06-01 12:22","friends":[1,2,3]}"#
        );
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test
