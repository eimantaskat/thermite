// ANCHOR: imports
use std::error::Error;

use futures_util::stream;
use serde::{Deserialize, Serialize};
use thermite::{
    App, EventError, EventSourceResponse, Header, Model, RouteError, Server, ServerSentEvent, get,
    routes,
};
// ANCHOR_END: imports

// ANCHOR: items
#[derive(Serialize, Model)]
struct Item {
    name: String,
    price: f64,
}

fn items() -> Vec<Item> {
    [
        ("Plumbus", 32.99),
        ("Portal Gun", 999.99),
        ("Meeseeks Box", 49.99),
    ]
    .into_iter()
    .map(|(name, price)| Item {
        name: name.to_owned(),
        price,
    })
    .collect()
}
// ANCHOR_END: items

// ANCHOR: stream
fn item_event(index: i64, item: &Item) -> Result<ServerSentEvent, EventError> {
    ServerSentEvent::new()
        .event("item")?
        .data(item)?
        .id(index.to_string())
}

#[get("/items/stream")]
async fn stream_items() -> EventSourceResponse {
    let events = items()
        .into_iter()
        .zip(0..)
        .map(|(item, index)| item_event(index, &item));
    EventSourceResponse::from_results(stream::iter(events))
}
// ANCHOR_END: stream

// ANCHOR: resume
#[derive(Deserialize, Model)]
struct Resume {
    last_event_id: Option<i64>,
}

#[get("/items/resumable")]
async fn resumable_items(Header(resume): Header<Resume>) -> EventSourceResponse {
    let start = resume
        .last_event_id
        .map_or(0, |seen| seen.saturating_add(1));
    let events = items()
        .into_iter()
        .zip(0..)
        .filter(move |(_, index)| *index >= start)
        .map(|(item, index)| item_event(index, &item));
    EventSourceResponse::from_results(stream::iter(events))
}
// ANCHOR_END: resume

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new().routes(routes![stream_items, resumable_items])
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
    use thermite::http::header::CONTENT_TYPE;

    use super::*;

    #[tokio::test]
    async fn resumes_after_the_last_event_seen() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client
            .get("/items/resumable")
            .header("last-event-id", "1")
            .send()
            .await?;

        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "text/event-stream; charset=utf-8"
        );
        assert_eq!(
            response.text()?,
            "event: item\ndata: {\"name\":\"Meeseeks Box\",\"price\":49.99}\nid: 2\n\n"
        );
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows these answers as they are
#[cfg(test)]
mod shown_tests {
    use thermite::TestClient;
    use thermite::http::header::CACHE_CONTROL;

    use super::*;

    #[tokio::test]
    async fn writes_each_item_as_an_event() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/items/stream").send().await?;

        assert_eq!(response.headers()[CACHE_CONTROL], "no-cache");
        assert_eq!(response.headers()["x-accel-buffering"], "no");
        assert_eq!(
            response.text()?,
            concat!(
                "event: item\n",
                "data: {\"name\":\"Plumbus\",\"price\":32.99}\n",
                "id: 0\n",
                "\n",
                "event: item\n",
                "data: {\"name\":\"Portal Gun\",\"price\":999.99}\n",
                "id: 1\n",
                "\n",
                "event: item\n",
                "data: {\"name\":\"Meeseeks Box\",\"price\":49.99}\n",
                "id: 2\n",
                "\n"
            )
        );
        client.stop().await;
        Ok(())
    }

    #[test]
    fn refuses_an_event_name_with_a_line_break() {
        let refused = ServerSentEvent::new().event("item\ndata: forged");

        assert!(refused.is_err());
    }
}
