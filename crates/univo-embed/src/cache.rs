//! The local model cache.
//!
//! Layout, under the cache root:
//!
//! ```text
//! models/<organization>/<name>/<commit>/        verified model files and univo-model.json
//! models/<organization>/<name>/refs/<revision>  commit a branch or tag resolved to
//! models/<organization>/<name>/<commit>.lock    held while that commit is downloaded
//! tmp/                                          staging directories
//! ```
//!
//! Downloads land in a staging directory, are verified there, and are then
//! published with one directory rename, so a reader sees either no model or
//! a complete, verified one. A lock per commit keeps two processes from
//! downloading the same model at once; the operating system releases it if
//! the holder dies.

use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use univo_core::{Error, Result};

use crate::files::{Digest, FileExpectation, decode_hex, verify_file};
use crate::hub::DownloadPlan;
use crate::spec::Repository;

/// Name of the manifest written last in a published model directory.
pub const MANIFEST: &str = "univo-model.json";
const MANIFEST_FORMAT: u64 = 1;

/// A model published in the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedModel {
    pub path: PathBuf,
    pub repository: Repository,
    /// Full commit hash.
    pub revision: String,
    pub files: Vec<FileExpectation>,
}

#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
    lock_timeout: Duration,
}

/// A download in progress: the lock on the model and the directory its
/// files are written to.
#[derive(Debug)]
pub struct Staging {
    plan: DownloadPlan,
    directory: PathBuf,
    destination: PathBuf,
    _lock: File,
}

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lock_timeout: Duration::from_secs(600),
        }
    }

    #[must_use]
    pub fn with_lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
    }

    /// `UNIVO_CACHE_DIR`, or the platform's cache directory: `%LOCALAPPDATA%`
    /// on Windows, `~/Library/Caches` on macOS, `$XDG_CACHE_HOME` or
    /// `~/.cache` elsewhere, each followed by `univo`.
    pub fn default_root() -> Option<PathBuf> {
        let var = |name| std::env::var_os(name).filter(|value| !value.is_empty());
        if let Some(root) = var("UNIVO_CACHE_DIR") {
            return Some(PathBuf::from(root));
        }
        if cfg!(windows) {
            return var("LOCALAPPDATA").map(|base| PathBuf::from(base).join("univo").join("cache"));
        }
        let home = var("HOME").map(PathBuf::from);
        if cfg!(target_os = "macos") {
            return home.map(|home| home.join("Library").join("Caches").join("univo"));
        }
        var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|home| home.join(".cache")))
            .map(|base| base.join("univo"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn repository_dir(&self, repository: &Repository) -> PathBuf {
        self.root
            .join("models")
            .join(&repository.organization)
            .join(&repository.name)
    }

    /// Finds a published model. A revision that is not a full commit hash,
    /// or no revision, is resolved through the reference recorded when it
    /// was downloaded.
    ///
    /// # Errors
    ///
    /// Returns `model.integrity` when a published directory has a missing or
    /// malformed manifest, or a file whose size changed.
    pub fn find(
        &self,
        repository: &Repository,
        revision: Option<&str>,
    ) -> Result<Option<CachedModel>> {
        let requested = revision.unwrap_or("main");
        let commit = if is_commit(requested) {
            requested.to_ascii_lowercase()
        } else {
            let reference = self.repository_dir(repository).join("refs").join(requested);
            match fs::read_to_string(reference) {
                Ok(commit) if is_commit(commit.trim()) => commit.trim().to_owned(),
                _ => return Ok(None),
            }
        };
        let path = self.repository_dir(repository).join(&commit);
        if !path.join(MANIFEST).is_file() {
            return Ok(None);
        }
        read_manifest(&path, repository, &commit).map(Some)
    }

    /// Locks the model of `plan` and creates its staging directory.
    ///
    /// Returns `Ok(None)` when another process published the model while
    /// this one waited for the lock.
    ///
    /// # Errors
    ///
    /// Returns `model.cache_locked` after the lock timeout and
    /// `model.unavailable` when the cache directory cannot be written.
    pub fn stage(&self, plan: &DownloadPlan) -> Result<Option<Staging>> {
        let model = plan.model();
        let unwritable = |error: std::io::Error| {
            Error::model_not_available(&model, &format!("the cache cannot be written: {error}"))
        };
        let repository_dir = self.repository_dir(&plan.repository);
        fs::create_dir_all(&repository_dir).map_err(unwritable)?;
        let lock_path = repository_dir.join(format!("{}.lock", plan.revision));
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(unwritable)?;
        let deadline = Instant::now() + self.lock_timeout;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(Error::model_cache_locked(
                        &model,
                        &lock_path.display().to_string(),
                    ));
                }
                Err(TryLockError::Error(error)) => return Err(unwritable(error)),
            }
        }

        let destination = repository_dir.join(&plan.revision);
        if destination.join(MANIFEST).is_file() {
            return Ok(None);
        }
        let directory = self.root.join("tmp").join(unique_name());
        fs::create_dir_all(&directory).map_err(unwritable)?;
        Ok(Some(Staging {
            plan: plan.clone(),
            directory,
            destination,
            _lock: lock,
        }))
    }

    /// Verifies every staged file and publishes the model.
    ///
    /// # Errors
    ///
    /// Returns `model.integrity` when a file is missing or differs from the
    /// plan, and `model.unavailable` when the cache cannot be written. The
    /// staging directory is removed in every case.
    pub fn publish(&self, staging: Staging) -> Result<CachedModel> {
        let result = self.publish_staged(&staging);
        let _ = fs::remove_dir_all(&staging.directory);
        // Dropping the staging releases the lock, after publication.
        drop(staging);
        result
    }

    fn publish_staged(&self, staging: &Staging) -> Result<CachedModel> {
        let plan = &staging.plan;
        let model = plan.model();
        for file in &plan.files {
            verify_file(
                &model,
                &staging.file_path(&file.expectation.name),
                &file.expectation,
            )?;
        }
        let unwritable = |error: std::io::Error| {
            Error::model_not_available(&model, &format!("the cache cannot be written: {error}"))
        };
        let files: Vec<FileExpectation> = plan
            .files
            .iter()
            .map(|file| file.expectation.clone())
            .collect();
        fs::write(
            staging.directory.join(MANIFEST),
            manifest_json(&plan.repository, &plan.revision, &files),
        )
        .map_err(unwritable)?;
        fs::rename(&staging.directory, &staging.destination).map_err(unwritable)?;

        if let Some(reference) = &plan.requested_revision
            && !is_commit(reference)
        {
            let refs = self.repository_dir(&plan.repository).join("refs");
            fs::create_dir_all(&refs).map_err(unwritable)?;
            let temporary = refs.join(format!(".{}", unique_name()));
            fs::write(&temporary, &plan.revision).map_err(unwritable)?;
            fs::rename(&temporary, refs.join(reference)).map_err(unwritable)?;
        }
        Ok(CachedModel {
            path: staging.destination.clone(),
            repository: plan.repository.clone(),
            revision: plan.revision.clone(),
            files,
        })
    }
}

impl Staging {
    /// Where the binding must write `file` before publishing.
    pub fn file_path(&self, file: &str) -> PathBuf {
        self.directory.join(file)
    }

    pub fn plan(&self) -> &DownloadPlan {
        &self.plan
    }
}

fn is_commit(revision: &str) -> bool {
    revision.len() == 40 && decode_hex::<20>(revision).is_some()
}

/// A name unique within this process and, with the process id and the
/// time, across processes sharing the cache.
fn unique_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn manifest_json(repository: &Repository, revision: &str, files: &[FileExpectation]) -> String {
    let files: Vec<serde_json::Value> = files
        .iter()
        .map(|file| {
            let (kind, hex) = match file.digest {
                Digest::Sha256(_) => ("sha256", file.digest.to_hex()),
                Digest::GitBlobSha1(_) => ("git-blob-sha1", file.digest.to_hex()),
            };
            serde_json::json!({"name": file.name, "size": file.size, kind: hex})
        })
        .collect();
    serde_json::json!({
        "format": MANIFEST_FORMAT,
        "repository": repository.to_string(),
        "revision": revision,
        "files": files,
    })
    .to_string()
}

/// Reads a published manifest and checks that every file still has its
/// recorded size; contents were verified before publication.
fn read_manifest(path: &Path, repository: &Repository, commit: &str) -> Result<CachedModel> {
    let model = format!("{repository}@{commit}");
    let malformed = |reason: &str| Error::model_integrity(&model, MANIFEST, reason);
    let text = fs::read_to_string(path.join(MANIFEST))
        .map_err(|_| malformed("the manifest cannot be read"))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| malformed("the manifest is not JSON"))?;
    if json["format"].as_u64() != Some(MANIFEST_FORMAT) || json["revision"] != commit {
        return Err(malformed("the manifest describes another format or commit"));
    }
    let entries = json["files"]
        .as_array()
        .ok_or_else(|| malformed("the manifest lists no files"))?;
    let mut files = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry["name"]
            .as_str()
            .ok_or_else(|| malformed("a file has no name"))?;
        let size = entry["size"]
            .as_u64()
            .ok_or_else(|| malformed("a file has no size"))?;
        let digest = entry["sha256"]
            .as_str()
            .and_then(decode_hex::<32>)
            .map(Digest::Sha256)
            .or_else(|| {
                entry["git-blob-sha1"]
                    .as_str()
                    .and_then(decode_hex::<20>)
                    .map(Digest::GitBlobSha1)
            })
            .ok_or_else(|| malformed("a file has no digest"))?;
        let actual = fs::metadata(path.join(name)).map(|metadata| metadata.len());
        if actual.ok() != Some(size) {
            return Err(Error::model_integrity(
                &model,
                name,
                "the cached file is missing or changed size since it was verified",
            ));
        }
        files.push(FileExpectation {
            name: name.to_owned(),
            size,
            digest,
        });
    }
    Ok(CachedModel {
        path: path.to_path_buf(),
        repository: repository.clone(),
        revision: commit.to_owned(),
        files,
    })
}
