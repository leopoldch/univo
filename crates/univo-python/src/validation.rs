//! Validation shared with the Python layer, before any model is resolved.

use pyo3::prelude::*;
use univo_core::Error;

use crate::errors::to_py_err;

#[pyfunction]
pub(crate) fn validate_model(py: Python<'_>, model: &str) -> PyResult<()> {
    univo_embed::ModelSpec::parse(model)
        .map(drop)
        .map_err(|e| to_py_err(py, &e))
}
#[pyfunction]
pub(crate) fn invalid_option(py: Python<'_>, name: &str, expected: &str) -> PyErr {
    to_py_err(py, &Error::invalid_option(name, expected))
}

/// Validate before any model resolution, using the same f32 range as search.
#[pyfunction]
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn validate_threshold(py: Python<'_>, min_score: f64) -> PyResult<f32> {
    let score = min_score as f32;
    if !(min_score.is_finite() && min_score > 0.0 && min_score <= 1.0 && score > 0.0) {
        return Err(to_py_err(py, &Error::invalid_threshold(score)));
    }
    Ok(score)
}
