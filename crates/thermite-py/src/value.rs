use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use thermite_core::Value;

pub(crate) fn to_python<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyAny>> {
    match value {
        Value::Null => Ok(py.None().into_bound(py)),
        Value::Bool(value) => value.into_bound_py_any(py),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                integer.into_bound_py_any(py)
            } else if let Some(integer) = number.as_u64() {
                integer.into_bound_py_any(py)
            } else {
                number.as_f64().into_bound_py_any(py)
            }
        }
        Value::String(value) => value.into_bound_py_any(py),
        Value::Array(items) => {
            let items = items
                .iter()
                .map(|item| to_python(py, item))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyList::new(py, items)?.into_any())
        }
        Value::Object(fields) => {
            let object = PyDict::new(py);
            for (name, value) in fields {
                object.set_item(name, to_python(py, value)?)?;
            }
            Ok(object.into_any())
        }
    }
}
