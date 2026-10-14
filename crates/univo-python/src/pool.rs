//! Worker threads of a Finder, started on first use.

use std::sync::{Arc, Mutex, PoisonError};

use pyo3::prelude::*;
use univo_core::Error;

use crate::errors::to_py_err;

/// Records from which `find` and `group` run on the worker pool.
pub(crate) const PARALLEL_MIN_RECORDS: usize = 512;

/// Worker threads started on first use: a Finder that never encodes nor
/// runs a large report never starts any.
#[derive(Debug)]
pub(crate) struct LazyPool {
    pub(crate) threads: usize,
    pool: Mutex<Option<Arc<rayon::ThreadPool>>>,
}

impl LazyPool {
    pub(crate) fn new(threads: usize) -> univo_core::Result<Self> {
        if threads == 0 {
            return Err(Error::invalid_option("threads", "a positive integer"));
        }
        Ok(Self {
            threads,
            pool: Mutex::new(None),
        })
    }

    /// The workers, started by the first caller.
    pub(crate) fn get(&self) -> univo_core::Result<Arc<rayon::ThreadPool>> {
        // Building cannot leave the slot half-written: recover a poisoned lock.
        let mut slot = self.pool.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pool) = slot.as_ref() {
            return Ok(Arc::clone(pool));
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.threads)
            .build()
            .map(Arc::new)
            .map_err(|_| Error::invalid_option("threads", "an available worker pool"))?;
        *slot = Some(Arc::clone(&pool));
        Ok(pool)
    }

    fn started(&self) -> bool {
        self.pool
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

/// The worker threads of one Finder, shared by all its indexes: the
/// persistent one and the temporary ones of `find` and `group`. Its threads
/// start on first use and are created once, so small calls pay no thread
/// creation.
#[derive(Debug)]
#[pyclass(frozen, module = "univo._univo", name = "WorkerPool")]
pub(crate) struct PyWorkerPool {
    pub(crate) pool: Arc<LazyPool>,
}

#[pymethods]
impl PyWorkerPool {
    #[new]
    fn new(py: Python<'_>, threads: usize) -> PyResult<Self> {
        LazyPool::new(threads)
            .map(|pool| Self {
                pool: Arc::new(pool),
            })
            .map_err(|e| to_py_err(py, &e))
    }

    #[getter]
    fn threads(&self) -> usize {
        self.pool.threads
    }

    /// Whether the worker threads have started.
    #[getter]
    fn started(&self) -> bool {
        self.pool.started()
    }
}

/// Threads this process may run at once: the cores it is allowed, within
/// CPU affinity and, on Linux, container quotas. One when unknown.
#[pyfunction]
pub(crate) fn available_threads() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

pub(crate) fn run<T: Send>(pool: Option<&rayon::ThreadPool>, work: impl FnOnce() -> T + Send) -> T {
    match pool {
        Some(pool) => pool.install(work),
        None => work(),
    }
}
