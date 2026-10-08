//! Text embedding with static `Model2Vec` models (ADR 0007).

use std::borrow::Cow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use univo_core::{Error, Result};

use crate::cache::CachedModel;
use crate::files::{FileExpectation, MODEL_FILES, sha256, verify_content};
use crate::spec::Repository;

/// Identifies the backend in descriptors; part of vector compatibility.
pub const BACKEND: &str = "model2vec-rs 0.3.0";

/// Where the loaded model came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSource {
    /// A verified copy of a hub model, at a commit, with its file digests.
    Hub {
        repository: Repository,
        revision: String,
        files: Vec<FileExpectation>,
    },
    /// A local directory supplied by the user, read as it is and identified
    /// by the SHA-256 of the files that were loaded.
    Local {
        path: PathBuf,
        files: Vec<FileExpectation>,
    },
}

/// What an embedder produces and where it comes from: everything that
/// decides whether two vectors are comparable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDescriptor {
    pub source: ModelSource,
    pub dimension: usize,
    pub backend: &'static str,
}

impl ModelDescriptor {
    /// Human-readable model name for messages.
    pub fn name(&self) -> String {
        match &self.source {
            ModelSource::Hub {
                repository,
                revision,
                ..
            } => format!("{repository}@{revision}"),
            ModelSource::Local { path, .. } => path.display().to_string(),
        }
    }
}

/// Unit vectors in one contiguous row-major buffer. A text without any
/// token the model knows has no semantic signal: its row is zero and it is
/// listed as absent, never silently normalized.
#[derive(Debug, Clone, PartialEq)]
pub struct Embeddings {
    data: Vec<f32>,
    dimension: usize,
    absent: Vec<usize>,
}

impl Embeddings {
    pub fn len(&self) -> usize {
        self.data.len().checked_div(self.dimension).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    pub fn row(&self, index: usize) -> &[f32] {
        &self.data[index * self.dimension..(index + 1) * self.dimension]
    }

    /// Rows without semantic signal, in increasing order.
    pub fn absent(&self) -> &[usize] {
        &self.absent
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.data
    }
}

/// Encodes texts into unit vectors; the substitution point between
/// embedding backends.
pub trait Embedder: Send + Sync {
    fn descriptor(&self) -> &ModelDescriptor;

    /// Encodes `texts` into one row each, in order.
    ///
    /// # Errors
    ///
    /// Returns `model.corrupt` when the model fails or produces a vector
    /// that is not finite or not of unit length.
    fn encode_batch(&self, texts: &[&str]) -> Result<Embeddings>;
}

/// A static embedding model: token vectors averaged, then normalized.
pub struct StaticEmbedder {
    model: model2vec_rs::model::StaticModel,
    descriptor: ModelDescriptor,
}

impl std::fmt::Debug for StaticEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticEmbedder")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

impl StaticEmbedder {
    /// Loads a model from the cache. Files are verified against the
    /// digests recorded at download time, in memory, so the bytes checked
    /// are the bytes used.
    ///
    /// # Errors
    ///
    /// Returns `model.integrity` when a file changed since it was
    /// downloaded; otherwise as [`StaticEmbedder::load_directory`].
    pub fn load_cached(model: &CachedModel) -> Result<Self> {
        Self::load(&model.path, Some(model))
    }

    /// Loads a model from a local directory supplied by the user.
    ///
    /// # Errors
    ///
    /// Returns `model.unavailable` when the directory does not exist,
    /// `model.unsupported_format` when a `Model2Vec` file is missing, and
    /// `model.corrupt` when the files cannot be read as a model.
    pub fn load_directory(directory: &Path) -> Result<Self> {
        Self::load(directory, None)
    }

    fn load(directory: &Path, cached: Option<&CachedModel>) -> Result<Self> {
        let name = cached.map_or_else(
            || directory.display().to_string(),
            |model| format!("{}@{}", model.repository, model.revision),
        );
        if !directory.is_dir() {
            return Err(Error::model_not_available(
                &name,
                "the directory does not exist",
            ));
        }
        let missing: Vec<_> = MODEL_FILES
            .iter()
            .filter(|file| !directory.join(file).is_file())
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(Error::unsupported_model_format(
                &name,
                &format!(
                    "{} missing; a Model2Vec model has {}",
                    missing.join(", "),
                    MODEL_FILES.join(", ")
                ),
            ));
        }

        let read = |file: &str| {
            std::fs::read(directory.join(file)).map_err(|error| {
                Error::corrupt_model(&name, &format!("cannot read {file}: {error}"))
            })
        };
        let contents = [
            read(MODEL_FILES[0])?,
            read(MODEL_FILES[1])?,
            read(MODEL_FILES[2])?,
        ];
        let [config, weights, tokenizer] = &contents;

        // Hashing the files takes about as long as building the model:
        // both run at once, the model is discarded unless the files verify,
        // and an integrity error takes precedence over a parsing error.
        let (source, model) = std::thread::scope(|scope| {
            let identify = scope.spawn(|| identify(&name, directory, cached, &contents));
            let model = without_length_settings(&name, tokenizer).and_then(|tokenizer| {
                guarded(&name, || {
                    model2vec_rs::model::StaticModel::from_bytes(
                        &*tokenizer,
                        weights,
                        config,
                        Some(true),
                    )
                })?
                .map_err(|error| Error::corrupt_model(&name, &format!("{error:#}")))
            });
            let source = identify
                .join()
                .unwrap_or_else(|_| Err(Error::corrupt_model(&name, "verification failed")));
            (source, model)
        });
        let source = source?;
        let model = model?;

        let probe = guarded(&name, || {
            model.encode_with_args(&["probe".to_owned()], None, 1)
        })?;
        let dimension = probe.first().map_or(0, Vec::len);
        if dimension == 0 {
            return Err(Error::corrupt_model(&name, "the model has no dimension"));
        }
        Ok(Self {
            model,
            descriptor: ModelDescriptor {
                source,
                dimension,
                backend: BACKEND,
            },
        })
    }
}

/// Verifies cached files against their recorded digests, or identifies a
/// local model by the SHA-256 of its files.
fn identify(
    name: &str,
    directory: &Path,
    cached: Option<&CachedModel>,
    contents: &[Vec<u8>],
) -> Result<ModelSource> {
    let Some(model) = cached else {
        return Ok(ModelSource::Local {
            path: directory.to_path_buf(),
            files: MODEL_FILES
                .iter()
                .zip(contents)
                .map(|(file, content)| FileExpectation {
                    name: (*file).to_owned(),
                    size: content.len() as u64,
                    digest: sha256(content),
                })
                .collect(),
        });
    };
    for (file, content) in MODEL_FILES.iter().zip(contents) {
        let expected = model
            .files
            .iter()
            .find(|expected| expected.name == *file)
            .ok_or_else(|| {
                Error::model_integrity(name, file, "the manifest does not list this file")
            })?;
        verify_content(name, content, expected)?;
    }
    Ok(ModelSource::Hub {
        repository: model.repository.clone(),
        revision: model.revision.clone(),
        files: model.files.clone(),
    })
}

/// The length settings of a `tokenizer.json`; everything else is skipped
/// without being built.
#[derive(serde::Deserialize)]
struct LengthSettings {
    #[serde(default)]
    truncation: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    padding: Option<serde::de::IgnoredAny>,
}

/// Removes the truncation and padding a tokenizer may configure. A static
/// model averages token vectors: truncation would drop tokens silently, and
/// padding would add pad tokens whose number depends on the other texts of
/// the batch.
fn without_length_settings<'a>(model: &str, tokenizer: &'a [u8]) -> Result<Cow<'a, [u8]>> {
    let corrupt =
        |error: serde_json::Error| Error::corrupt_model(model, &format!("tokenizer.json: {error}"));
    let settings: LengthSettings = serde_json::from_slice(tokenizer).map_err(corrupt)?;
    if settings.truncation.is_none() && settings.padding.is_none() {
        return Ok(Cow::Borrowed(tokenizer));
    }
    let mut json: serde_json::Value = serde_json::from_slice(tokenizer).map_err(corrupt)?;
    for key in ["truncation", "padding"] {
        json[key] = serde_json::Value::Null;
    }
    serde_json::to_vec(&json).map(Cow::Owned).map_err(corrupt)
}

/// Runs backend code and turns a panic into `model.corrupt`, so that a
/// malformed model cannot unwind into a binding.
fn guarded<T>(model: &str, work: impl FnOnce() -> T) -> Result<T> {
    catch_unwind(AssertUnwindSafe(work))
        .map_err(|_| Error::corrupt_model(model, "the model failed while processing input"))
}

/// Texts encoded per backend call: large enough to keep the tokenizer's
/// threads busy, small enough to bound the backend's per-text allocations.
const ENCODE_CHUNK: usize = 4096;

/// Tolerance on the norm of a unit vector computed in `f32`.
const UNIT_TOLERANCE: f32 = 1e-3;

impl Embedder for StaticEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn encode_batch(&self, texts: &[&str]) -> Result<Embeddings> {
        let name = self.descriptor.name();
        let dimension = self.descriptor.dimension;
        let mut data = Vec::with_capacity(texts.len() * dimension);
        let mut absent = Vec::new();
        // The backend returns one allocation per text: encoding by chunks
        // keeps only one chunk of them alive next to the result.
        for (chunk_index, chunk) in texts.chunks(ENCODE_CHUNK).enumerate() {
            let owned: Vec<String> = chunk.iter().map(|text| (*text).to_owned()).collect();
            // Static models average token vectors, so no length limit
            // applies: nothing is truncated.
            let vectors = guarded(&name, || self.model.encode_with_args(&owned, None, 1024))?;
            if vectors.len() != chunk.len() {
                return Err(Error::corrupt_model(&name, "a text has no vector"));
            }
            for (offset, vector) in vectors.iter().enumerate() {
                if vector.len() != dimension {
                    return Err(Error::corrupt_model(
                        &name,
                        "a vector has the wrong dimension",
                    ));
                }
                let norm_squared: f32 = vector.iter().map(|value| value * value).sum();
                if !norm_squared.is_finite() {
                    return Err(Error::corrupt_model(&name, "a vector is not finite"));
                }
                if norm_squared == 0.0 {
                    absent.push(chunk_index * ENCODE_CHUNK + offset);
                } else if (norm_squared.sqrt() - 1.0).abs() > UNIT_TOLERANCE {
                    return Err(Error::corrupt_model(
                        &name,
                        "a vector is not of unit length",
                    ));
                }
                data.extend_from_slice(vector);
            }
        }
        Ok(Embeddings {
            data,
            dimension,
            absent,
        })
    }
}
