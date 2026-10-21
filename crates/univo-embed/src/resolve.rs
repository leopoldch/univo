//! From a model reference to a model on disk: the cache first, then a
//! download through the binding's [`Transport`], never the network when
//! offline.

use std::path::{Path, PathBuf};

use univo_core::{Error, Result};

use crate::cache::{Cache, CachedModel, Stage};
use crate::hub::{
    DEFAULT_ENDPOINT, DEFAULT_MAX_DOWNLOAD_BYTES, DownloadPlan, metadata_url, plan_from_metadata,
    plan_pinned, validate_endpoint,
};
use crate::model::StaticEmbedder;
use crate::registry::default_model;
use crate::spec::{ModelSpec, Repository};

/// Largest metadata response accepted from a hub.
const MAX_METADATA_BYTES: u64 = 32 << 20;

/// A failed request, as reported by a [`Transport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError {
    /// HTTP status, when the server answered.
    pub status: Option<u16>,
    /// What went wrong, without credentials.
    pub reason: String,
}

/// HTTP GET requests, performed by a binding with its language's standard
/// library. Implementations follow redirects, apply a timeout,
/// retry a bounded number of times, and send nothing but the URL.
pub trait Transport {
    /// Writes the body of `url` to `destination`, failing once it exceeds
    /// `max_bytes`.
    ///
    /// # Errors
    ///
    /// Returns the reason the request failed.
    fn fetch(&self, url: &str, destination: &Path, max_bytes: u64) -> Result<(), TransportError>;

    /// Returns the body of `url` as text, failing once it exceeds
    /// `max_bytes`.
    ///
    /// # Errors
    ///
    /// Returns the reason the request failed.
    fn fetch_text(&self, url: &str, max_bytes: u64) -> Result<String, TransportError>;

    /// Called while another process holds the model's lock; returning
    /// false stops waiting.
    fn keep_waiting(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveOptions {
    /// Hub or mirror base URL.
    pub endpoint: String,
    /// When set, nothing is downloaded and the transport is never called.
    pub offline: bool,
    /// Largest total size of one model's files.
    pub max_download_bytes: u64,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_ENDPOINT.to_owned(),
            offline: false,
            max_download_bytes: DEFAULT_MAX_DOWNLOAD_BYTES,
        }
    }
}

/// A model available on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A directory supplied by the user.
    Local(PathBuf),
    /// A verified model in the cache.
    Cached(CachedModel),
}

impl Resolved {
    /// Loads the model.
    ///
    /// # Errors
    ///
    /// See [`StaticEmbedder::load_cached`] and
    /// [`StaticEmbedder::load_directory`].
    pub fn load(&self) -> Result<StaticEmbedder> {
        match self {
            Self::Local(path) => StaticEmbedder::load_directory(path),
            Self::Cached(model) => StaticEmbedder::load_cached(model),
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Local(path) => path,
            Self::Cached(model) => &model.path,
        }
    }
}

/// Finds `spec` in the cache, or downloads it unless `options.offline`.
///
/// A cached model is used without any request, including a branch resolved
/// by an earlier download: pin a commit to change revision.
///
/// # Errors
///
/// Returns `model.unavailable` when the model is not cached and cannot be
/// downloaded, `model.download_failed` when a request fails, and the errors
/// of [`plan_from_metadata`], [`Cache::stage`] and [`Cache::publish`].
pub fn resolve(
    spec: &ModelSpec,
    cache: &Cache,
    options: &ResolveOptions,
    transport: &dyn Transport,
) -> Result<Resolved> {
    let (repository, revision) = match spec {
        ModelSpec::Local(path) => {
            // Absolute, so that messages and descriptors do not depend on
            // the working directory; the file system is not consulted.
            let path = std::path::absolute(path).unwrap_or_else(|_| path.clone());
            return Ok(Resolved::Local(path));
        }
        ModelSpec::Default => {
            let pinned = default_model();
            (pinned.repository, Some(pinned.revision.to_owned()))
        }
        ModelSpec::Hub {
            repository,
            revision,
        } => (repository.clone(), revision.clone()),
    };
    if let Some(model) = cache.find(&repository, revision.as_deref())? {
        return Ok(Resolved::Cached(model));
    }
    if options.offline {
        return Err(Error::model_not_available(
            &spec.to_string(),
            &format!(
                "offline mode is on and the model is not in the cache at {}",
                cache.root().display()
            ),
        ));
    }

    let endpoint = validate_endpoint(&options.endpoint)?;
    let plan = if matches!(spec, ModelSpec::Default) {
        plan_pinned(&endpoint, &default_model())
    } else {
        let metadata = fetch_metadata(&endpoint, &repository, revision.as_deref(), transport)?;
        plan_from_metadata(
            &endpoint,
            &repository,
            revision.as_deref(),
            &metadata,
            options.max_download_bytes,
        )?
    };
    download(cache, &plan, options.max_download_bytes, transport).map(Resolved::Cached)
}

fn fetch_metadata(
    endpoint: &str,
    repository: &Repository,
    revision: Option<&str>,
    transport: &dyn Transport,
) -> Result<String> {
    let revision = revision.unwrap_or(crate::hub::DEFAULT_REVISION);
    let model = format!("{repository}@{revision}");
    transport
        .fetch_text(
            &metadata_url(endpoint, repository, revision),
            MAX_METADATA_BYTES,
        )
        .map_err(|error| match error.status {
            Some(401 | 403 | 404) => Error::model_not_available(
                &model,
                &format!(
                    "{endpoint} has no public repository {repository} at revision {revision} (HTTP {})",
                    error.status.unwrap_or_default()
                ),
            ),
            _ => Error::model_download_failed(&model, "metadata", &error.reason),
        })
}

fn download(
    cache: &Cache,
    plan: &DownloadPlan,
    max_bytes: u64,
    transport: &dyn Transport,
) -> Result<CachedModel> {
    // Checked for every plan, pinned ones included, before any request.
    if plan.total_bytes() > max_bytes {
        return Err(Error::model_too_large(
            &plan.model(),
            plan.total_bytes(),
            max_bytes,
        ));
    }
    let staging = match cache.stage_with(plan, &|| transport.keep_waiting())? {
        Stage::Published(model) => return Ok(model),
        Stage::Download(staging) => staging,
    };
    for file in &plan.files {
        let name = &file.expectation.name;
        transport
            .fetch(&file.url, &staging.file_path(name), file.expectation.size)
            .map_err(|error| Error::model_download_failed(&plan.model(), name, &error.reason))?;
    }
    cache.publish(staging)
}
