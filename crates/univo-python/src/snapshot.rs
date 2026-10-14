//! Snapshots opened from Python: configuration first, then data.

use pyo3::prelude::*;
use univo_core::Error;

use crate::cache::PyEmbeddingCache;
use crate::errors::to_py_err;
use crate::index::NativeIndex;
use crate::models::Model;
use crate::pool::PyWorkerPool;
use crate::schema::PySchema;

/// Keeps an opened generation stable between configuration and data loading.
#[derive(Debug)]
#[pyclass(name = "Snapshot", module = "univo._univo")]
pub(crate) struct PySnapshot {
    snapshot: Option<univo_engine::snapshot::Snapshot>,
}

#[pymethods]
impl PySnapshot {
    #[new]
    #[allow(clippy::needless_pass_by_value)] // PyO3 extracts an owned path.
    fn new(py: Python<'_>, path: std::path::PathBuf, max_bytes: u64) -> PyResult<Self> {
        py.detach(|| univo_engine::snapshot::Snapshot::open(&path, max_bytes))
            .map(|snapshot| Self {
                snapshot: Some(snapshot),
            })
            .map_err(|e| to_py_err(py, &e))
    }
    #[getter]
    fn configuration(&self, py: Python<'_>) -> PyResult<String> {
        self.snapshot
            .as_ref()
            .map(|s| s.configuration().to_string())
            .ok_or_else(|| to_py_err(py, &Error::snapshot_invalid("snapshot already consumed")))
    }
    fn check_capacity(&self, py: Python<'_>, max_records: usize) -> PyResult<()> {
        self.snapshot
            .as_ref()
            .ok_or_else(|| to_py_err(py, &Error::snapshot_invalid("snapshot already consumed")))?
            .check_capacity(max_records)
            .map_err(|e| to_py_err(py, &e))
    }
    #[pyo3(signature = (schema, max_records, model=None, pool=None, cache=None, ann=None))]
    #[allow(clippy::too_many_arguments)] // PyO3 mirrors index construction.
    fn restore(
        &mut self,
        py: Python<'_>,
        schema: &PySchema,
        max_records: usize,
        model: Option<&Model>,
        pool: Option<&PyWorkerPool>,
        cache: Option<&PyEmbeddingCache>,
        ann: Option<(usize, usize)>,
    ) -> PyResult<NativeIndex> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| to_py_err(py, &Error::snapshot_invalid("snapshot already consumed")))?;
        let generation = snapshot.generation();
        py.detach(|| {
            let (index, pool) =
                NativeIndex::build(&schema.schema, max_records, model, pool, cache, ann)?;
            let index = snapshot.restore(index)?;
            Ok(NativeIndex::assemble(index, generation, max_records, pool))
        })
        .map_err(|e: Error| to_py_err(py, &e))
    }
}
