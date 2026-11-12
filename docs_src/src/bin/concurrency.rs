// ANCHOR: imports
use std::error::Error;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thermite::{
    App, HttpError, Json, Model, Query, RouteError, Server, State, StatusCode, get, routes,
};
// ANCHOR_END: imports

// ANCHOR: order
async fn get_burgers(count: i64) -> String {
    tokio::time::sleep(Duration::from_millis(100)).await;
    format!("{count} burgers")
}

async fn get_drinks(count: i64) -> String {
    tokio::time::sleep(Duration::from_millis(100)).await;
    format!("{count} drinks")
}

#[derive(Serialize, Model)]
struct Order {
    burgers: String,
    drinks: String,
}

#[get("/order")]
async fn order() -> Json<Order> {
    let (burgers, drinks) = tokio::join!(get_burgers(2), get_drinks(2));
    Json(Order { burgers, drinks })
}
// ANCHOR_END: order

// ANCHOR: blocking
fn is_prime(number: i64) -> bool {
    if number < 2 {
        return false;
    }
    let mut divisor = 2;
    while divisor * divisor <= number {
        if number % divisor == 0 {
            return false;
        }
        divisor += 1;
    }
    true
}

fn count_primes(limit: i64) -> i64 {
    let mut count = 0;
    for number in 2..=limit {
        if is_prime(number) {
            count += 1;
        }
    }
    count
}

#[derive(Deserialize, Model)]
struct Limit {
    limit: i64,
}

#[derive(Serialize, Model)]
struct Primes {
    limit: i64,
    count: i64,
}

#[get("/primes")]
async fn primes(Query(Limit { limit }): Query<Limit>) -> Result<Json<Primes>, thermite::Error> {
    if limit > 10_000_000 {
        let detail = "limit must be at most 10000000";
        return Err(HttpError::new(StatusCode::UNPROCESSABLE_ENTITY, detail).into());
    }
    let count = tokio::task::spawn_blocking(move || count_primes(limit)).await?;
    Ok(Json(Primes { limit, count }))
}
// ANCHOR_END: blocking

// ANCHOR: lock
#[derive(Default)]
struct Visits(Mutex<i64>);

#[derive(Serialize, Model)]
struct Visit {
    visit: i64,
}

#[get("/visit")]
async fn visit(State(visits): State<Visits>) -> Json<Visit> {
    let visit = {
        let mut count = visits.0.lock().unwrap_or_else(PoisonError::into_inner);
        *count += 1;
        *count
    };
    // stands in for a call to another service
    tokio::time::sleep(Duration::from_millis(10)).await;
    Json(Visit { visit })
}
// ANCHOR_END: lock

// ANCHOR: app
fn app() -> Result<App, RouteError> {
    App::new()
        .state(Visits::default())
        .routes(routes![order, primes, visit])
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

    #[tokio::test]
    async fn the_order_holds_both_answers() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/order").send().await?;

        assert_eq!(
            response.text()?,
            r#"{"burgers":"2 burgers","drinks":"2 drinks"}"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn primes_are_counted_off_the_async_threads() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/primes?limit=100").send().await?;

        assert_eq!(response.text()?, r#"{"limit":100,"count":25}"#);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_limit_too_large_is_refused() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let response = client.get("/primes?limit=10000001").send().await?;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response.text()?,
            r#"{"detail":"limit must be at most 10000000"}"#
        );
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn each_visit_is_counted_once() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app()?).await?;

        let first = client.get("/visit").send().await?;
        let second = client.get("/visit").send().await?;

        assert_eq!(first.text()?, r#"{"visit":1}"#);
        assert_eq!(second.text()?, r#"{"visit":2}"#);
        client.stop().await;
        Ok(())
    }
}
// ANCHOR_END: test
