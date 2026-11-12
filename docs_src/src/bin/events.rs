// a resource is written the way users write one, as an async fn that may not await
#![allow(clippy::unused_async_trait_impl)]

// ANCHOR: imports
use std::env;
use std::error::Error;
use std::io;

use serde::{Deserialize, Serialize};
use thermite::{App, Json, Model, Query, Resource, RouteError, Server, State, get, routes};
// ANCHOR_END: imports

// ANCHOR: resource
fn fake_answer_to_everything_ml_model(x: f64) -> f64 {
    x * 42.0
}

struct Settings {
    model_name: String,
}

struct MlModel {
    predict: fn(f64) -> f64,
}

impl Resource for MlModel {
    type Needs = State<Settings>;
    type Error = io::Error;

    async fn start(State(settings): State<Settings>) -> Result<Self, io::Error> {
        match settings.model_name.as_str() {
            "answer_to_everything" => Ok(Self {
                predict: fake_answer_to_everything_ml_model,
            }),
            other => Err(io::Error::other(format!("no model named {other}"))),
        }
    }
}
// ANCHOR_END: resource

// ANCHOR: app
#[derive(Deserialize, Model)]
struct Input {
    x: f64,
}

#[derive(Serialize, Model)]
struct Prediction {
    result: f64,
}

#[get("/predict")]
async fn predict(Query(input): Query<Input>, State(model): State<MlModel>) -> Json<Prediction> {
    Json(Prediction {
        result: (model.predict)(input.x),
    })
}

fn app(settings: Settings) -> Result<App, RouteError> {
    App::new()
        .state(settings)
        .resource::<MlModel>()
        .routes(routes![predict])
}
// ANCHOR_END: app

// ANCHOR: main
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let settings = Settings {
        model_name: env::var("MODEL_NAME").unwrap_or_else(|_| "answer_to_everything".to_owned()),
    };
    let server = Server::bind("127.0.0.1:8000").await?;
    server.serve(app(settings)?).await?;
    Ok(())
}
// ANCHOR_END: main

// ANCHOR: test
#[cfg(test)]
mod tests {
    use thermite::{ServerError, TestClient};

    use super::*;

    fn settings(model_name: &str) -> Settings {
        Settings {
            model_name: model_name.to_owned(),
        }
    }

    #[tokio::test]
    async fn predicts_with_the_model_loaded_at_startup() -> Result<(), Box<dyn Error>> {
        let client = TestClient::start(app(settings("answer_to_everything"))?).await?;

        let response = client.get("/predict?x=2").send().await?;

        assert_eq!(response.text()?, r#"{"result":84.0}"#);
        client.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn does_not_start_without_its_model() -> Result<(), Box<dyn Error>> {
        let started = TestClient::start(app(settings("unknown"))?).await;

        let Err(ServerError::Startup(error)) = started else {
            return Err("the app started".into());
        };
        assert_eq!(
            error.to_string(),
            "the resource events::MlModel failed to start"
        );
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("no model named unknown")
        );
        Ok(())
    }
}
// ANCHOR_END: test

// the page shows this error as main prints it
#[cfg(test)]
mod shown_tests {
    use thermite::TestClient;

    use super::*;

    #[tokio::test]
    async fn main_prints_the_resource_and_its_error() -> Result<(), Box<dyn Error>> {
        let settings = Settings {
            model_name: "unknown".to_owned(),
        };

        let Err(error) = TestClient::start(app(settings)?).await else {
            return Err("the app started".into());
        };

        assert_eq!(
            format!("Error: {error:?}"),
            concat!(
                r#"Error: Startup(Resource { resource: "events::MlModel", "#,
                r#"source: Custom { kind: Other, error: "no model named unknown" } })"#
            )
        );
        Ok(())
    }
}
