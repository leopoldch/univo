//! Native extension module `univo._univo`.
//!
//! Converts Python inputs and outputs only; all matching logic lives in the
//! Rust crates so that every binding shares the same decisions.

mod cache;
mod errors;
mod index;
mod models;
mod pool;
mod schema;
mod snapshot;
mod validation;

use pyo3::prelude::*;

#[pymodule]
fn _univo(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", univo_core::VERSION)?;
    module.add("ERROR_KINDS", errors::kind_names())?;
    module.add("SCORING_VERSION", univo_core::SCORING_VERSION)?;
    module.add("SEARCH_VERSION", univo_engine::SEARCH_VERSION)?;
    module.add("NORMALIZER_VERSION", univo_core::NORMALIZER_VERSION)?;
    module.add_class::<models::Model>()?;
    module.add_function(wrap_pyfunction!(models::load_model, module)?)?;
    module.add_class::<schema::PySchema>()?;
    module.add_class::<index::NativeIndex>()?;
    module.add_class::<cache::PyEmbeddingCache>()?;
    module.add_class::<snapshot::PySnapshot>()?;
    module.add_class::<pool::PyWorkerPool>()?;
    module.add_function(wrap_pyfunction!(pool::available_threads, module)?)?;
    module.add_function(wrap_pyfunction!(validation::validate_model, module)?)?;
    module.add_function(wrap_pyfunction!(validation::invalid_option, module)?)?;
    module.add_function(wrap_pyfunction!(validation::validate_threshold, module)?)?;
    Ok(())
}
