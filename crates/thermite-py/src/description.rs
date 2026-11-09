use pyo3::IntoPyObjectExt;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList};
use thermite_core as core;

use crate::value::to_python;

#[pyclass(frozen, from_py_object, module = "thermite._thermite")]
#[derive(Debug, Clone)]
pub(crate) struct Schema(pub(crate) core::Schema);

#[pymethods]
impl Schema {
    #[staticmethod]
    fn string() -> Self {
        Self(core::Schema::string())
    }

    #[staticmethod]
    fn integer() -> Self {
        Self(core::Schema::integer())
    }

    #[staticmethod]
    fn number() -> Self {
        Self(core::Schema::number())
    }

    #[staticmethod]
    fn boolean() -> Self {
        Self(core::Schema::boolean())
    }

    #[staticmethod]
    fn any() -> Self {
        Self(core::Schema::any())
    }

    #[staticmethod]
    fn array(items: Schema) -> Self {
        Self(core::Schema::array(items.0))
    }

    #[staticmethod]
    fn model(name: &str, fields: Vec<Field>) -> Self {
        Self(core::Schema::model(
            name,
            fields.into_iter().map(|field| field.0),
        ))
    }

    #[staticmethod]
    fn reference(name: &str) -> Self {
        Self(core::Schema::reference(name))
    }

    // a handler's answer as JSON, validated and filtered, or the errors of one that does not match
    fn validate_response<'py>(&self, py: Python<'py>, body: &[u8]) -> PyResult<Bound<'py, PyAny>> {
        let validated = py.detach(|| {
            let value: core::Value = serde_json::from_slice(body)?;
            Ok::<_, serde_json::Error>(
                self.0
                    .validate_response(value)
                    .map(|validated| validated.to_string()),
            )
        });
        match validated.map_err(|error| PyValueError::new_err(error.to_string()))? {
            Ok(validated) => PyBytes::new(py, validated.as_bytes()).into_bound_py_any(py),
            Err(error) => {
                let errors = error
                    .errors()
                    .iter()
                    .map(|error| to_python(py, error))
                    .collect::<PyResult<Vec<_>>>()?;
                PyList::new(py, errors)?.into_bound_py_any(py)
            }
        }
    }
}

#[pyclass(frozen, from_py_object, module = "thermite._thermite")]
#[derive(Debug, Clone)]
pub(crate) struct Field(core::Field);

#[pymethods]
impl Field {
    #[staticmethod]
    fn required(name: &str, schema: Schema) -> Self {
        Self(core::Field::new(name, schema.0))
    }

    #[staticmethod]
    fn optional(name: &str, schema: Schema) -> Self {
        Self(core::Field::new(name, schema.0).optional())
    }

    #[staticmethod]
    fn with_default_factory(name: &str, schema: Schema) -> Self {
        Self(core::Field::new(name, schema.0).with_default_factory())
    }

    #[staticmethod]
    fn with_default(name: &str, schema: Schema, default_json: &str) -> PyResult<Self> {
        let default: core::Value = serde_json::from_str(default_json)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(Self(core::Field::new(name, schema.0).with_default(default)))
    }
}

#[pyclass(frozen, from_py_object, module = "thermite._thermite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParameterLocation {
    Path,
    Query,
    Header,
    Cookie,
}

#[pyclass(frozen, from_py_object, module = "thermite._thermite")]
#[derive(Debug, Clone)]
pub(crate) struct Parameter {
    pub(crate) argument: String,
    pub(crate) location: core::ParameterLocation,
    pub(crate) name: String,
    pub(crate) description: core::Parameter,
}

#[pymethods]
impl Parameter {
    #[new]
    fn new(argument: String, location: ParameterLocation, field: Field) -> Self {
        let location = match location {
            ParameterLocation::Path => core::ParameterLocation::Path,
            ParameterLocation::Query => core::ParameterLocation::Query,
            ParameterLocation::Header => core::ParameterLocation::Header,
            ParameterLocation::Cookie => core::ParameterLocation::Cookie,
        };
        Self {
            argument,
            location,
            name: field.0.name().to_owned(),
            description: core::Parameter::new(location, field.0),
        }
    }
}
