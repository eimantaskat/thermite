use std::mem;
use std::sync::Arc;
use std::time::Instant;
use std::vec;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use serde_json::map;
use thermite_core::Value;

use super::plan::{ModelId, Plan, ShapeKind};
use crate::value::to_python;

// reading the clock for every value would cost more than some values take to build
const VALUES_BETWEEN_CHECKS: u32 = 64;

#[derive(Debug)]
pub(crate) struct Deadline {
    // None never comes
    end: Option<Instant>,
    unchecked: u32,
}

impl Deadline {
    pub(crate) fn new(end: Option<Instant>) -> Self {
        Self {
            end,
            unchecked: VALUES_BETWEEN_CHECKS,
        }
    }

    fn reached(&mut self) -> bool {
        let Some(end) = self.end else {
            return false;
        };
        self.unchecked -= 1;
        if self.unchecked > 0 {
            return false;
        }
        self.unchecked = VALUES_BETWEEN_CHECKS;
        Instant::now() >= end
    }
}

#[derive(Debug)]
pub(crate) enum Progress {
    Built(Py<PyAny>),
    Paused(Build),
}

// turns a validated body into the handler's argument, a piece at a time, freeing the JSON as it goes
#[derive(Debug)]
pub(crate) struct Build {
    plan: Arc<Plan>,
    // what is being built now, with what it will be part of below it
    top: Frame,
    below: Vec<Frame>,
}

#[derive(Debug)]
enum Frame {
    Root {
        value: Option<Value>,
        built: Py<PyAny>,
    },
    List {
        item: Arc<ShapeKind>,
        values: vec::IntoIter<Value>,
        built: Vec<Py<PyAny>>,
    },
    Dict {
        values: map::IntoIter,
        key: String,
        built: Py<PyDict>,
    },
    Model {
        model: ModelId,
        values: map::IntoIter,
        // how many fields were taken, the last of which is being built
        taken: usize,
        positional: Vec<Py<PyAny>>,
        keywords: Option<Py<PyDict>>,
    },
}

enum Opened {
    Frame(Frame),
    Built(Py<PyAny>),
}

impl Build {
    pub(crate) fn new(py: Python<'_>, plan: Arc<Plan>, body: Value) -> Self {
        Self {
            plan,
            top: Frame::Root {
                value: Some(body),
                built: py.None(),
            },
            below: Vec::new(),
        }
    }

    pub(crate) fn resume(mut self, py: Python<'_>, deadline: &mut Deadline) -> PyResult<Progress> {
        loop {
            if deadline.reached() {
                return Ok(Progress::Paused(self));
            }
            if let Some((value, shape)) = self.top.next_value(&self.plan)? {
                match open(py, value, shape)? {
                    Opened::Frame(frame) => self.below.push(mem::replace(&mut self.top, frame)),
                    Opened::Built(built) => self.top.add(py, &self.plan, built)?,
                }
                continue;
            }
            let Some(parent) = self.below.pop() else {
                return Ok(Progress::Built(self.top.finish(py, &self.plan)?));
            };
            let built = mem::replace(&mut self.top, parent).finish(py, &self.plan)?;
            self.top.add(py, &self.plan, built)?;
        }
    }
}

impl Frame {
    // the next value to build for this frame, or None once all of them are built
    fn next_value(&mut self, plan: &Plan) -> PyResult<Option<(Value, ShapeKind)>> {
        Ok(match self {
            Self::Root { value, .. } => value.take().map(|value| (value, plan.root.clone())),
            Self::List { item, values, .. } => {
                values.next().map(|value| (value, ShapeKind::clone(item)))
            }
            Self::Dict { values, key, .. } => values.next().map(|(name, value)| {
                *key = name;
                (value, ShapeKind::Any)
            }),
            Self::Model {
                model,
                values,
                taken,
                ..
            } => {
                let fields = &plan.model(*model).fields;
                let Some((name, value)) = values.next() else {
                    return if *taken == fields.len() {
                        Ok(None)
                    } else {
                        Err(unexpected_layout())
                    };
                };
                let Some(expected) = fields.get(*taken).filter(|field| field.name == name) else {
                    return Err(unexpected_layout());
                };
                *taken += 1;
                Some((value, expected.shape.clone()))
            }
        })
    }

    fn add(&mut self, py: Python<'_>, plan: &Plan, value: Py<PyAny>) -> PyResult<()> {
        match self {
            Self::Root { built, .. } => *built = value,
            Self::List { built, .. } => built.push(value),
            Self::Dict { key, built, .. } => built.bind(py).set_item(mem::take(key), value)?,
            Self::Model {
                model,
                taken,
                positional,
                keywords,
                ..
            } => match &plan.model(*model).fields[*taken - 1].keyword {
                None => positional.push(value),
                Some(name) => keywords
                    .get_or_insert_with(|| PyDict::new(py).unbind())
                    .bind(py)
                    .set_item(name, value)?,
            },
        }
        Ok(())
    }

    fn finish(self, py: Python<'_>, plan: &Plan) -> PyResult<Py<PyAny>> {
        match self {
            Self::Root { built, .. } => Ok(built),
            Self::List { built, .. } => Ok(PyList::new(py, built)?.into_any().unbind()),
            Self::Dict { built, .. } => Ok(built.into_any()),
            Self::Model {
                model,
                positional,
                keywords,
                ..
            } => {
                let keywords = keywords.as_ref().map(|keywords| keywords.bind(py));
                let built = plan
                    .model(model)
                    .model
                    .bind(py)
                    .call(PyTuple::new(py, positional)?, keywords)?;
                Ok(built.unbind())
            }
        }
    }
}

fn open(py: Python<'_>, value: Value, shape: ShapeKind) -> PyResult<Opened> {
    Ok(match (shape, value) {
        (ShapeKind::List(item), Value::Array(values)) => Opened::Frame(list(item, values)),
        (ShapeKind::Model(model), Value::Object(values)) => Opened::Frame(Frame::Model {
            model,
            positional: Vec::with_capacity(values.len()),
            values: values.into_iter(),
            taken: 0,
            keywords: None,
        }),
        (_, Value::Array(values)) => Opened::Frame(list(Arc::new(ShapeKind::Any), values)),
        (_, Value::Object(values)) => Opened::Frame(Frame::Dict {
            built: PyDict::new(py).unbind(),
            values: values.into_iter(),
            key: String::new(),
        }),
        (_, value) => Opened::Built(to_python(py, &value)?.unbind()),
    })
}

fn list(item: Arc<ShapeKind>, values: Vec<Value>) -> Frame {
    Frame::List {
        item,
        built: Vec::with_capacity(values.len()),
        values: values.into_iter(),
    }
}

// validation gives a model exactly the fields its schema lists, in that order, which is the plan's
fn unexpected_layout() -> PyErr {
    PyRuntimeError::new_err("the validated body does not have the fields of its model in order")
}
