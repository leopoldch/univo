//! Helpers shared by the integration tests of this crate.

// Each test binary uses a subset of the helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The tiny model built by `tools/build_tiny_model.py`.
pub(crate) fn tiny_model() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/models/tiny")
}

/// A fresh, empty directory removed when dropped.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "univo-embed-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Copies the tiny model into a fresh directory.
pub(crate) fn tiny_model_copy(label: &str) -> TempDir {
    let directory = TempDir::new(label);
    for file in univo_embed::MODEL_FILES {
        std::fs::copy(tiny_model().join(file), directory.path().join(file)).unwrap();
    }
    directory
}
