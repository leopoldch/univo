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

/// The default model (ADR 0021): `sentence-transformers/static-similarity-mrl-multilingual-v1`
/// reduced to int8 and 128 dimensions, Apache-2.0, 50 languages. The files
/// are rebuilt byte for byte by `tools/build_default_model.py`, which checks
/// the same checksums.
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
            organization: "univo-models".to_owned(),
            name: "static-similarity-mrl-multilingual-int8-128".to_owned(),
        },
        // Replaced by the commit of the published repository.
        revision: "0000000000000000000000000000000000000000",
        files: vec![
            file(
                "config.json",
                79,
                "78f117ff0287650054a2a596313e72fe2e3dec4e4cc0d6348985a6e50cc19028",
            ),
            file(
                "model.safetensors",
                13_552_600,
                "73e3c5e5496b5ddf9070238139b701839d763a479ab8023ec25bc951f780b6fc",
            ),
            file(
                "tokenizer.json",
                1_715_270,
                "88d69c30a2a4395cf169631e47f3c4fbb95d300650028d541ec531ed18eb0ff1",
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
