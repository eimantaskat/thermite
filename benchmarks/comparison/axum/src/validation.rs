// FastAPI's 422 for axum: serde alone for a valid body, and on failure a walk over the JSON that lists
// every error with pydantic's types and messages and accepts what it coerces, such as "9.5" for a float
use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

pub type Location = Vec<Value>;

pub trait Validate: Sized {
    fn validate(validator: &mut Validator, value: &Value, location: &mut Location) -> Option<Self>;
}

impl<T: Validate> Validate for Vec<T> {
    fn validate(validator: &mut Validator, value: &Value, location: &mut Location) -> Option<Self> {
        validator.list(value, location, T::validate)
    }
}

#[derive(Default)]
pub struct Validator {
    errors: Vec<Value>,
}

impl Validator {
    fn fail(&mut self, kind: &str, location: &Location, message: &str, input: &Value) {
        self.errors
            .push(json!({"type": kind, "loc": location, "msg": message, "input": input}));
    }

    pub fn object(&mut self, value: &Value, location: &Location) -> Option<()> {
        if value.is_object() {
            return Some(());
        }
        self.fail(
            "model_attributes_type",
            location,
            "Input should be a valid dictionary or object to extract fields from",
            value,
        );
        None
    }

    pub fn field<T>(
        &mut self,
        object: &Value,
        location: &mut Location,
        name: &str,
        check: impl FnOnce(&mut Self, &Value, &mut Location) -> Option<T>,
    ) -> Option<T> {
        location.push(Value::from(name));
        let checked = if let Some(value) = object.get(name) {
            check(self, value, location)
        } else {
            self.fail("missing", location, "Field required", object);
            None
        };
        location.pop();
        checked
    }

    pub fn list<T>(
        &mut self,
        value: &Value,
        location: &mut Location,
        mut check: impl FnMut(&mut Self, &Value, &mut Location) -> Option<T>,
    ) -> Option<Vec<T>> {
        let Some(elements) = value.as_array() else {
            self.fail("list_type", location, "Input should be a valid list", value);
            return None;
        };
        let mut checked = Vec::with_capacity(elements.len());
        let mut failed = false;
        for (index, element) in elements.iter().enumerate() {
            location.push(Value::from(index));
            match check(self, element, location) {
                Some(valid) => checked.push(valid),
                None => failed = true,
            }
            location.pop();
        }
        (!failed).then_some(checked)
    }

    pub fn string(&mut self, value: &Value, location: &mut Location) -> Option<String> {
        if let Some(text) = value.as_str() {
            return Some(text.to_owned());
        }
        self.fail(
            "string_type",
            location,
            "Input should be a valid string",
            value,
        );
        None
    }

    pub fn float(&mut self, value: &Value, location: &mut Location) -> Option<f64> {
        let parsed = match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => {
                let Ok(float) = text.trim().parse::<f64>() else {
                    self.fail(
                        "float_parsing",
                        location,
                        "Input should be a valid number, unable to parse string as a number",
                        value,
                    );
                    return None;
                };
                Some(float)
            }
            _ => None,
        };
        match parsed {
            Some(float) if float.is_finite() => Some(float),
            Some(_) => {
                self.fail(
                    "finite_number",
                    location,
                    "Input should be a finite number",
                    value,
                );
                None
            }
            None => {
                self.fail(
                    "float_type",
                    location,
                    "Input should be a valid number",
                    value,
                );
                None
            }
        }
    }

    fn into_response(self) -> Response {
        unprocessable(self.errors)
    }
}

fn unprocessable(errors: Vec<Value>) -> Response {
    let mut body = Map::new();
    body.insert("detail".to_owned(), Value::Array(errors));
    (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(body)).into_response()
}

// serde_json's message without its position, which goes into the location as a byte offset on the first line
fn json_invalid(error: &serde_json::Error) -> Response {
    let message = error.to_string();
    let message = message
        .rsplit_once(" at line ")
        .map_or(message.as_str(), |(message, _)| message);
    let position = error.column().saturating_sub(1);
    unprocessable(vec![json!({
        "type": "json_invalid",
        "loc": ["body", position],
        "msg": "JSON decode error",
        "input": {},
        "ctx": {"error": message},
    })])
}

pub struct Validated<T>(pub T);

impl<T, State> FromRequest<State> for Validated<T>
where
    T: DeserializeOwned + Validate,
    State: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &State) -> Result<Self, Self::Rejection> {
        let body = Bytes::from_request(request, state)
            .await
            .map_err(IntoResponse::into_response)?;
        if let Ok(valid) = serde_json::from_slice(&body) {
            return Ok(Self(valid));
        }
        let value: Value = serde_json::from_slice(&body).map_err(|error| json_invalid(&error))?;
        let mut validator = Validator::default();
        let mut location = vec![Value::from("body")];
        match T::validate(&mut validator, &value, &mut location) {
            Some(valid) => Ok(Self(valid)),
            None => Err(validator.into_response()),
        }
    }
}
