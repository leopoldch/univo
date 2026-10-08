//! Models pinned by this release. Their revisions and checksums are part of
//! the release: a change requires a new version and invalidates caches and
//! indexes built with the previous model.

use crate::files::{Digest, FileExpectation, decode_hex};
use crate::spec::Repository;

/// A model pinned to a commit, with the expected content of every file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedModel {
    pub repository: Repository,
    /// Full commit hash.
    pub revision: &'static str,
    pub files: Vec<FileExpectation>,
}

/// The default model, proposed by ADR 0007: `minishlab/potion-base-8M`,
/// MIT, 256 dimensions, English. Checksums computed from the files of this
/// commit and checked against the hub's published LFS hash and Git blob ids.
///
/// # Panics
///
/// Never: the pinned checksums are constants, decoded in this crate's tests.
pub fn default_model() -> PinnedModel {
    let file = |name: &str, size: u64, sha256: &str| FileExpectation {
        name: name.to_owned(),
        size,
        digest: Digest::Sha256(decode_hex(sha256).expect("pinned checksums are valid hex")),
    };
    PinnedModel {
        repository: Repository {
            organization: "minishlab".to_owned(),
            name: "potion-base-8M".to_owned(),
        },
        revision: "bf8b056651a2c21b8d2565580b8569da283cab23",
        files: vec![
            file(
                "config.json",
                202,
                "2a6ac0e9aaa356a68a5688070db78fc3a464fefe85d2f06a1905ce3718687553",
            ),
            file(
                "model.safetensors",
                30_236_760,
                "f65d0f325faadc1e121c319e2faa41170d3fa07d8c89abd48ca5358d9a223de2",
            ),
            file(
                "tokenizer.json",
                683_666,
                "e67e803f624fb4d67dea1c730d06e1067e1b14d830e2c2202569e3ef0f70bb50",
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::MODEL_FILES;

    #[test]
    fn default_model_pins_every_model_file() {
        let model = default_model();
        assert_eq!(model.revision.len(), 40);
        let names: Vec<_> = model.files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, MODEL_FILES);
    }
}
