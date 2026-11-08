use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyString, PyType};

use crate::description::Schema;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModelId(usize);

#[derive(Debug, Clone)]
pub(crate) enum ShapeKind {
    // whatever the JSON holds, as dicts, lists and scalars
    Any,
    List(Arc<ShapeKind>),
    Model(ModelId),
}

impl ShapeKind {
    fn model(&self) -> Option<ModelId> {
        match self {
            Self::Any => None,
            Self::List(item) => item.model(),
            Self::Model(model) => Some(*model),
        }
    }
}

#[pyclass(frozen, from_py_object, module = "thermite._thermite")]
#[derive(Debug, Clone)]
pub(crate) struct Shape(ShapeKind);

#[pymethods]
impl Shape {
    #[staticmethod]
    fn any() -> Self {
        Self(ShapeKind::Any)
    }

    #[staticmethod]
    fn list(item: Shape) -> Self {
        Self(ShapeKind::List(Arc::new(item.0)))
    }

    // the model's place in the list given to `Body`, which lets a model contain itself
    #[staticmethod]
    fn model(index: usize) -> Self {
        Self(ShapeKind::Model(ModelId(index)))
    }
}

#[pyclass(frozen, from_py_object, module = "thermite._thermite")]
#[derive(Debug, Clone)]
pub(crate) struct FieldShape {
    name: String,
    shape: Shape,
    keyword_only: bool,
}

#[pymethods]
impl FieldShape {
    #[new]
    fn new(name: String, shape: Shape, keyword_only: bool) -> Self {
        Self {
            name,
            shape,
            keyword_only,
        }
    }
}

// a dataclass with its `__init__` arguments, in the order the schema lists them
#[pyclass(frozen, module = "thermite._thermite")]
#[derive(Debug)]
pub(crate) struct ModelShape {
    model: Py<PyType>,
    fields: Vec<FieldShape>,
}

#[pymethods]
impl ModelShape {
    #[new]
    fn new(model: Py<PyType>, fields: Vec<FieldShape>) -> Self {
        Self { model, fields }
    }
}

#[derive(Debug)]
pub(crate) struct ModelPlan {
    pub(crate) model: Py<PyType>,
    pub(crate) fields: Vec<FieldPlan>,
}

#[derive(Debug)]
pub(crate) struct FieldPlan {
    pub(crate) name: String,
    pub(crate) shape: ShapeKind,
    // None for a field passed by position
    pub(crate) keyword: Option<Py<PyString>>,
}

// how to build the handler's body argument from a validated body
#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) argument: Py<PyString>,
    pub(crate) root: ShapeKind,
    models: Vec<ModelPlan>,
}

impl Plan {
    pub(crate) fn model(&self, model: ModelId) -> &ModelPlan {
        &self.models[model.0]
    }
}

// the body argument of a route: what validates it and what builds it
#[pyclass(frozen, module = "thermite._thermite")]
#[derive(Debug)]
pub(crate) struct Body {
    pub(crate) schema: Schema,
    pub(crate) plan: Arc<Plan>,
}

#[pymethods]
impl Body {
    #[new]
    fn new(
        py: Python<'_>,
        argument: &str,
        schema: Schema,
        shape: Shape,
        models: Vec<Bound<'_, ModelShape>>,
    ) -> PyResult<Self> {
        let models: Vec<ModelPlan> = models
            .into_iter()
            .map(|model| {
                let model = model.get();
                ModelPlan {
                    model: model.model.clone_ref(py),
                    fields: model
                        .fields
                        .iter()
                        .map(|field| FieldPlan {
                            name: field.name.clone(),
                            shape: field.shape.0.clone(),
                            keyword: field
                                .keyword_only
                                .then(|| PyString::intern(py, &field.name).unbind()),
                        })
                        .collect(),
                }
            })
            .collect();
        let shapes = std::iter::once(&shape.0).chain(
            models
                .iter()
                .flat_map(|model| model.fields.iter().map(|field| &field.shape)),
        );
        if let Some(ModelId(index)) = shapes
            .filter_map(ShapeKind::model)
            .find(|model| model.0 >= models.len())
        {
            return Err(PyValueError::new_err(format!(
                "model {index} is out of range for {} models",
                models.len()
            )));
        }
        Ok(Self {
            schema,
            plan: Arc::new(Plan {
                argument: PyString::intern(py, argument).unbind(),
                root: shape.0,
                models,
            }),
        })
    }
}
