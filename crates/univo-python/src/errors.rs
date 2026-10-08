//! Conversion of core errors into the exceptions of `univo.errors`.

use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyModule};
use univo_core::{Detail, Error, ErrorCode, RecordId};

/// Names of the error kinds; `univo.errors` defines `<name>Error` for each.
pub(crate) fn kind_names() -> Vec<&'static str> {
    let mut names: Vec<_> = ErrorCode::ALL
        .iter()
        .map(|code| code.kind().name())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

static ERRORS: PyOnceLock<Py<PyModule>> = PyOnceLock::new();

/// The exception for `error`, with every part copied from the core.
pub(crate) fn to_py_err(py: Python<'_>, error: &Error) -> PyErr {
    match build(py, error) {
        Ok(exception) => PyErr::from_value(exception),
        Err(failure) => failure,
    }
}

fn build<'py>(py: Python<'py>, error: &Error) -> PyResult<Bound<'py, PyAny>> {
    let module = ERRORS
        .get_or_try_init(py, || py.import("univo.errors").map(Bound::unbind))?
        .bind(py);
    let class = module.getattr(format!("{}Error", error.kind().name()))?;

    let details = PyDict::new(py);
    for (key, value) in error.details() {
        match value {
            Detail::Text(text) => details.set_item(key, text)?,
            Detail::Count(count) => details.set_item(key, count)?,
            Detail::Number(number) => details.set_item(key, number)?,
        }
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("code", error.code().as_str())?;
    kwargs.set_item("hint", error.hint())?;
    kwargs.set_item("path", error.path().map(ToString::to_string))?;
    kwargs.set_item("details", details)?;
    kwargs.set_item("documentation_url", error.documentation_url())?;
    if let Some(record) = error.record() {
        kwargs.set_item("record_index", record.index)?;
        match &record.id {
            Some(RecordId::Str(id)) => kwargs.set_item("record_id", &**id)?,
            Some(RecordId::Int(id)) => kwargs.set_item("record_id", id)?,
            None => {}
        }
    }
    class.call((error.to_string(),), Some(&kwargs))
}
