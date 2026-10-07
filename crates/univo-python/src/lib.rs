//! Native extension module `univo._univo`.
//!
//! Converts Python inputs and outputs only; all matching logic lives in the
//! Rust crates so that every binding shares the same decisions.

use pyo3::prelude::*;

#[pymodule]
fn _univo(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", univo_core::VERSION)?;
    Ok(())
}
