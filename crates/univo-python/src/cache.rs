//! The embedding cache shared by the indexes of a Finder.

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::errors::to_py_err;
use crate::models::Model;

#[pyclass(name = "EmbeddingCache", module = "univo._univo")]
pub(crate) struct PyEmbeddingCache {
    pub(crate) cache: Arc<univo_engine::EmbeddingCache>,
}

#[pymethods]
impl PyEmbeddingCache {
    #[new]
    fn new(model: &Model, max_bytes: usize) -> Self {
        use univo_embed::Embedder;
        Self {
            cache: Arc::new(univo_engine::EmbeddingCache::new(
                model.embedder.descriptor().clone(),
                max_bytes,
            )),
        }
    }

    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let stats = self.cache.stats().map_err(|e| to_py_err(py, &e))?;
        let out = PyDict::new(py);
        for (key, value) in [
            ("embedding_cache_hits", stats.hits),
            ("embedding_cache_misses", stats.misses),
            ("embedding_cache_evictions", stats.evictions),
            ("embedding_cache_bytes", stats.bytes),
            ("embedding_cache_entries", stats.entries),
        ] {
            out.set_item(key, value)?;
        }
        Ok(out)
    }
}
