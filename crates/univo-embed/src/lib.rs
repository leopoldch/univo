//! Local embedding models for Univo.
//!
//! - [`spec`]: model references written by users.
//! - [`registry`]: models pinned by this release.
//! - [`hub`]: download plans for the Hugging Face hub and its mirrors.
//! - [`cache`]: the verified local cache.
//! - [`model`]: static `Model2Vec` inference behind the [`Embedder`] trait.
//! - [`resolve`]: from a reference to a model on disk, through the cache or
//!   a binding's [`Transport`].
//!
//! This crate performs no network access: bindings download the files a
//! plan lists with their language's standard library.

pub mod cache;
pub mod files;
pub mod hub;
pub mod model;
pub mod registry;
pub mod resolve;
pub mod spec;

pub use cache::{Cache, CachedModel, Stage, Staging};
pub use files::{Digest, FileExpectation, MODEL_FILES};
pub use hub::{
    DEFAULT_ENDPOINT, DEFAULT_MAX_DOWNLOAD_BYTES, DEFAULT_REVISION, DownloadPlan, PlannedFile,
};
pub use model::{Embedder, Embeddings, ModelDescriptor, ModelSource, StaticEmbedder};
pub use resolve::{ResolveOptions, Resolved, Transport, TransportError, resolve};
pub use spec::{ModelSpec, Repository};
