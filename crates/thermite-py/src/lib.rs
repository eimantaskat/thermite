use pyo3::prelude::*;
use thermite_core::Router;

#[pyclass(module = "thermite")]
#[derive(Debug, Default)]
struct App {
    #[expect(dead_code, reason = "step 23 adds routes to it")]
    router: Router,
}

#[pymethods]
impl App {
    #[new]
    fn new() -> Self {
        Self::default()
    }
}

#[pymodule]
mod _thermite {
    #[pymodule_export]
    use super::App;
}
