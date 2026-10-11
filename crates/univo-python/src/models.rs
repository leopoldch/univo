//! Model resolution and encoding for `univo.models`. Downloads go through
//! a Python transport object; everything else runs in Rust, detached from
//! the interpreter.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use univo_core::Error;
use univo_embed::hub::endpoint_from_env;
use univo_embed::{
    Cache, Digest, Embedder, ModelSource, ModelSpec, ResolveOptions, StaticEmbedder, Transport,
    TransportError, resolve,
};

use crate::errors::to_py_err;

/// A transport implemented in Python by `univo._transport`.
///
/// An exception that is not an `Exception`, such as `KeyboardInterrupt`,
/// stops the resolution and is raised again unchanged.
struct PyTransport {
    object: Py<PyAny>,
    interruption: Mutex<Option<PyErr>>,
}

impl PyTransport {
    fn failure(&self, py: Python<'_>, error: PyErr) -> TransportError {
        if !error.is_instance_of::<PyException>(py) {
            if let Ok(mut interruption) = self.interruption.lock() {
                interruption.get_or_insert(error);
            }
            return TransportError {
                status: None,
                reason: "interrupted".to_owned(),
            };
        }
        let value = error.value(py);
        let status = value
            .getattr("status")
            .ok()
            .and_then(|status| status.extract::<u16>().ok());
        let reason = value
            .getattr("reason")
            .ok()
            .and_then(|reason| reason.extract::<String>().ok())
            .unwrap_or_else(|| error.to_string());
        TransportError { status, reason }
    }

    fn take_interruption(&self) -> Option<PyErr> {
        self.interruption
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }
}

impl Transport for PyTransport {
    fn fetch(
        &self,
        url: &str,
        destination: &std::path::Path,
        max_bytes: u64,
    ) -> Result<(), TransportError> {
        Python::attach(|py| {
            self.object
                .bind(py)
                .call_method1("fetch", (url, destination, max_bytes))
                .map(drop)
                .map_err(|error| self.failure(py, error))
        })
    }

    fn fetch_text(&self, url: &str, max_bytes: u64) -> Result<String, TransportError> {
        Python::attach(|py| {
            self.object
                .bind(py)
                .call_method1("fetch_text", (url, max_bytes))
                .and_then(|text| text.extract::<String>())
                .map_err(|error| self.failure(py, error))
        })
    }

    fn keep_waiting(&self) -> bool {
        Python::attach(|py| match py.check_signals() {
            Ok(()) => true,
            Err(error) => {
                self.failure(py, error);
                false
            }
        })
    }
}

/// A loaded embedding model.
#[pyclass(frozen, module = "univo._univo")]
pub(crate) struct Model {
    pub(crate) embedder: Arc<StaticEmbedder>,
    path: PathBuf,
}

fn digest_text(digest: &Digest) -> String {
    match digest {
        Digest::Sha256(_) => format!("sha256:{}", digest.to_hex()),
        Digest::GitBlobSha1(_) => format!("git-blob-sha1:{}", digest.to_hex()),
    }
}

#[pymethods]
impl Model {
    #[getter]
    fn name(&self) -> String {
        self.embedder.descriptor().name()
    }

    #[getter]
    fn source(&self) -> &'static str {
        match self.embedder.descriptor().source {
            ModelSource::Hub { .. } => "hub",
            ModelSource::Local { .. } => "local",
        }
    }

    #[getter]
    fn repository(&self) -> Option<String> {
        match &self.embedder.descriptor().source {
            ModelSource::Hub { repository, .. } => Some(repository.to_string()),
            ModelSource::Local { .. } => None,
        }
    }

    #[getter]
    fn revision(&self) -> Option<&str> {
        match &self.embedder.descriptor().source {
            ModelSource::Hub { revision, .. } => Some(revision),
            ModelSource::Local { .. } => None,
        }
    }

    #[getter]
    fn path(&self) -> PathBuf {
        self.path.clone()
    }

    #[getter]
    fn dimension(&self) -> usize {
        self.embedder.descriptor().dimension
    }

    #[getter]
    fn backend(&self) -> &'static str {
        self.embedder.descriptor().backend
    }

    /// `(name, size, digest)` of each model file.
    #[getter]
    fn files(&self) -> Vec<(String, u64, String)> {
        let (ModelSource::Hub { files, .. } | ModelSource::Local { files, .. }) =
            &self.embedder.descriptor().source;
        files
            .iter()
            .map(|file| (file.name.clone(), file.size, digest_text(&file.digest)))
            .collect()
    }

    /// Encodes `texts` into row-major native-endian `float32` vectors, and
    /// the positions of texts without any known token.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "PyO3 converts the list into an owned vector"
    )]
    fn encode<'py>(
        &self,
        py: Python<'py>,
        texts: Vec<String>,
    ) -> PyResult<(Bound<'py, PyBytes>, Vec<usize>)> {
        let embeddings = py
            .detach(|| {
                let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
                self.embedder.encode_batch(&texts)
            })
            .map_err(|error| to_py_err(py, &error))?;
        let values = embeddings.as_slice();
        // Written straight into the Python object: one copy, no
        // intermediate buffer.
        let bytes = PyBytes::new_with(py, std::mem::size_of_val(values), |buffer| {
            for (chunk, value) in buffer.as_chunks_mut::<4>().0.iter_mut().zip(values) {
                *chunk = value.to_ne_bytes();
            }
            Ok(())
        })?;
        Ok((bytes, embeddings.absent().to_vec()))
    }

    fn __repr__(&self) -> String {
        format!(
            "<univo model {} ({} dimensions)>",
            self.name(),
            self.dimension()
        )
    }
}

/// Resolves `model` through the cache, downloading it with `transport`
/// unless `offline`, and loads it.
#[pyfunction]
#[pyo3(signature = (model, *, transport, hub_url=None, offline=false, cache_dir=None))]
pub(crate) fn load_model(
    py: Python<'_>,
    model: &str,
    transport: Py<PyAny>,
    hub_url: Option<String>,
    offline: bool,
    cache_dir: Option<PathBuf>,
) -> PyResult<Model> {
    let spec = ModelSpec::parse(model).map_err(|error| to_py_err(py, &error))?;
    let root = match cache_dir.or_else(Cache::default_root) {
        Some(root) => root,
        None if matches!(spec, ModelSpec::Local(_)) => PathBuf::new(),
        None => {
            let error = Error::model_not_available(
                model,
                "no cache directory is known: set UNIVO_CACHE_DIR or pass cache_dir",
            );
            return Err(to_py_err(py, &error));
        }
    };
    let options = ResolveOptions {
        endpoint: hub_url.unwrap_or_else(endpoint_from_env),
        offline,
        ..ResolveOptions::default()
    };
    let transport = PyTransport {
        object: transport,
        interruption: Mutex::new(None),
    };
    let result = py.detach(|| {
        let resolved = resolve(&spec, &Cache::new(root), &options, &transport)?;
        let embedder = resolved.load()?;
        Ok::<_, Error>(Model {
            embedder: Arc::new(embedder),
            path: resolved.path().to_path_buf(),
        })
    });
    if let Some(interruption) = transport.take_interruption() {
        return Err(interruption);
    }
    result.map_err(|error| to_py_err(py, &error))
}
