//! Helpers shared by the integration tests of this crate.

// Each test binary uses a subset of the helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha1::Digest as _;
use univo_embed::{MODEL_FILES, Repository};

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

pub(crate) const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

pub(crate) fn repository() -> Repository {
    Repository {
        organization: "univo-test".into(),
        name: "tiny".into(),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Hub metadata for the tiny model at [`COMMIT`]: the weights as an LFS file with a
/// SHA-256, the JSON files as Git blobs with a SHA-1, as the hub publishes.
pub(crate) fn tiny_metadata() -> String {
    let siblings: Vec<String> = MODEL_FILES
        .iter()
        .map(|name| {
            let content = std::fs::read(tiny_model().join(name)).unwrap();
            let size = content.len();
            if name.ends_with(".safetensors") {
                let sha256 = hex(&sha2::Sha256::digest(&content));
                format!(
                    r#"{{"rfilename": "{name}", "size": {size}, "lfs": {{"sha256": "{sha256}", "size": {size}}}}}"#
                )
            } else {
                let mut hasher = sha1::Sha1::new();
                hasher.update(format!("blob {size}\0").as_bytes());
                hasher.update(&content);
                let blob = hex(&hasher.finalize());
                format!(r#"{{"rfilename": "{name}", "size": {size}, "blobId": "{blob}"}}"#)
            }
        })
        .collect();
    format!(
        r#"{{"sha": "{COMMIT}", "siblings": [{}]}}"#,
        siblings.join(",")
    )
}
