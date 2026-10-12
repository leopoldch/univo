//! Native extension module `univo._univo`.
//!
//! Converts Python inputs and outputs only; all matching logic lives in the
//! Rust crates so that every binding shares the same decisions.

mod errors;
mod finder;
mod models;

use pyo3::prelude::*;

#[pymodule]
fn _univo(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", univo_core::VERSION)?;
    module.add("ERROR_KINDS", errors::kind_names())?;
    module.add_class::<models::Model>()?;
    module.add_function(wrap_pyfunction!(models::load_model, module)?)?;
    module.add_class::<finder::PySchema>()?;
    module.add_class::<finder::NativeIndex>()?;
    module.add_class::<finder::PyEmbeddingCache>()?;
    module.add_function(wrap_pyfunction!(finder::validate_model, module)?)?;
    module.add_function(wrap_pyfunction!(finder::invalid_option, module)?)?;
    module.add_function(wrap_pyfunction!(finder::validate_threshold, module)?)?;
    Ok(())
}
