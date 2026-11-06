mod app;
mod description;
mod event_loop;
mod route;
mod server;

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

create_exception!(thermite, RouteError, PyException);

#[pymodule]
mod _thermite {
    #[pymodule_export]
    use super::RouteError;
    #[pymodule_export]
    use super::app::CoreApp;
    #[pymodule_export]
    use super::description::{Field, Parameter, ParameterLocation, Schema};
    #[pymodule_export]
    use super::route::Respond;
    #[pymodule_export]
    use super::server::{PythonServer, TestServer};
}
