//! Download plans for models hosted on the Hugging Face hub or a compatible
//! mirror.
//!
//! This module builds URLs and validates published metadata; it performs no
//! network access. Bindings fetch the URLs with their language's standard
//! library, which uses the system's certificates and proxies, and hand the
//! responses back for verification (ADR 0008).

use univo_core::{Error, Result};

use crate::files::{Digest, FileExpectation, MODEL_FILES, decode_hex};
use crate::registry::PinnedModel;
use crate::spec::Repository;

pub const DEFAULT_ENDPOINT: &str = "https://huggingface.co";

/// Largest total download accepted by default: the multilingual static model
/// of the U-004 spike, about 1 GB, fits with room to spare.
pub const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 2 << 30;

/// A file to fetch and the content it must have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    pub url: String,
    pub expectation: FileExpectation,
}

/// Everything needed to download one model at one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadPlan {
    pub repository: Repository,
    /// Full commit hash.
    pub revision: String,
    /// The revision as requested, such as a branch, recorded to resolve it
    /// offline later.
    pub requested_revision: Option<String>,
    pub files: Vec<PlannedFile>,
}

impl DownloadPlan {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.expectation.size).sum()
    }

    pub fn model(&self) -> String {
        format!("{}@{}", self.repository, self.revision)
    }
}

/// Validates an endpoint: an absolute `https://` or `http://` URL without
/// credentials, query or fragment, returned without its trailing slash.
///
/// # Errors
///
/// Returns `model.invalid_spec` for any other string.
pub fn validate_endpoint(endpoint: &str) -> Result<String> {
    let trimmed = endpoint.trim_end_matches('/');
    let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
    else {
        return Err(Error::invalid_model_spec(
            endpoint,
            "the hub URL must start with https:// or http://",
        ));
    };
    if rest.is_empty() || rest.contains(['@', '?', '#', ' ']) {
        return Err(Error::invalid_model_spec(
            endpoint,
            "the hub URL must not contain credentials, a query or a fragment",
        ));
    }
    Ok(trimmed.to_owned())
}

/// URL of the metadata of `repository` at `revision`, with file sizes and
/// digests.
pub fn metadata_url(endpoint: &str, repository: &Repository, revision: &str) -> String {
    format!("{endpoint}/api/models/{repository}/revision/{revision}?blobs=true")
}

fn file_url(endpoint: &str, repository: &Repository, revision: &str, file: &str) -> String {
    format!("{endpoint}/{repository}/resolve/{revision}/{file}")
}

/// The plan of a pinned model; no metadata request is needed.
pub fn plan_pinned(endpoint: &str, model: &PinnedModel) -> DownloadPlan {
    DownloadPlan {
        repository: model.repository.clone(),
        revision: model.revision.to_owned(),
        requested_revision: None,
        files: model
            .files
            .iter()
            .map(|expectation| PlannedFile {
                url: file_url(
                    endpoint,
                    &model.repository,
                    model.revision,
                    &expectation.name,
                ),
                expectation: expectation.clone(),
            })
            .collect(),
    }
}

/// Builds a plan from the hub's metadata response for `repository`.
///
/// Files are pinned to the commit the metadata reports, so a branch that
/// moves during the download cannot mix two versions. Each file must have a
/// published digest: the LFS SHA-256 for large files, the Git blob SHA-1
/// otherwise.
///
/// # Errors
///
/// Returns `model.unsupported_format` when a model file is missing,
/// `model.integrity` when the metadata is malformed or reports another
/// commit than requested, and `model.too_large` above `max_bytes`.
pub fn plan_from_metadata(
    endpoint: &str,
    repository: &Repository,
    requested_revision: Option<&str>,
    metadata: &str,
    max_bytes: u64,
) -> Result<DownloadPlan> {
    let model = match requested_revision {
        Some(revision) => format!("{repository}@{revision}"),
        None => repository.to_string(),
    };
    let malformed = |reason: &str| Error::model_integrity(&model, "metadata", reason);
    let json: serde_json::Value =
        serde_json::from_str(metadata).map_err(|_| malformed("the metadata is not JSON"))?;
    let commit = json["sha"]
        .as_str()
        .filter(|sha| sha.len() == 40 && decode_hex::<20>(sha).is_some())
        .ok_or_else(|| malformed("the metadata has no commit hash"))?
        .to_ascii_lowercase();
    if let Some(revision) = requested_revision
        && revision.len() == 40
        && !revision.eq_ignore_ascii_case(&commit)
    {
        return Err(malformed("the metadata describes another commit"));
    }
    let siblings = json["siblings"]
        .as_array()
        .ok_or_else(|| malformed("the metadata lists no files"))?;

    let mut files = Vec::with_capacity(MODEL_FILES.len());
    for name in MODEL_FILES {
        let sibling = siblings
            .iter()
            .find(|sibling| sibling["rfilename"] == name)
            .ok_or_else(|| {
                Error::unsupported_model_format(
                    &model,
                    &format!(
                        "the repository has no {name}; Univo runs Model2Vec models made of {}",
                        MODEL_FILES.join(", ")
                    ),
                )
            })?;
        let lfs = &sibling["lfs"];
        let (size, digest) = if lfs.is_object() {
            let size = lfs["size"].as_u64();
            let digest = lfs["sha256"]
                .as_str()
                .and_then(decode_hex::<32>)
                .map(Digest::Sha256);
            (size, digest)
        } else {
            let size = sibling["size"].as_u64();
            let digest = sibling["blobId"]
                .as_str()
                .and_then(decode_hex::<20>)
                .map(Digest::GitBlobSha1);
            (size, digest)
        };
        let (Some(size), Some(digest)) = (size, digest) else {
            return Err(malformed(&format!(
                "{name} has no published size or digest"
            )));
        };
        files.push(PlannedFile {
            url: file_url(endpoint, repository, &commit, name),
            expectation: FileExpectation {
                name: name.to_owned(),
                size,
                digest,
            },
        });
    }

    let plan = DownloadPlan {
        repository: repository.clone(),
        revision: commit,
        requested_revision: requested_revision.map(str::to_owned),
        files,
    };
    if plan.total_bytes() > max_bytes {
        return Err(Error::model_too_large(
            &model,
            plan.total_bytes(),
            max_bytes,
        ));
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use univo_core::ErrorCode;

    fn repository() -> Repository {
        Repository {
            organization: "org".into(),
            name: "model".into(),
        }
    }

    const COMMIT: &str = "bf8b056651a2c21b8d2565580b8569da283cab23";

    fn metadata(extra_size: u64) -> String {
        format!(
            r#"{{"sha": "{COMMIT}", "siblings": [
                {{"rfilename": "config.json", "size": 202, "blobId": "7df26884a1aaaefbd7a30b37c32a25477cfb4c0e"}},
                {{"rfilename": "model.safetensors", "size": 30236760, "blobId": "c1009c715b593a264bfd06c93b600e786889cf7a",
                  "lfs": {{"sha256": "f65d0f325faadc1e121c319e2faa41170d3fa07d8c89abd48ca5358d9a223de2", "size": {}}}}},
                {{"rfilename": "tokenizer.json", "size": 683666, "blobId": "3e511f68ccf95c33b9ffd214a94c6d25bdb3034f"}},
                {{"rfilename": "README.md", "size": 10, "blobId": "0000000000000000000000000000000000000000"}}
            ]}}"#,
            30_236_760 + extra_size
        )
    }

    #[test]
    fn plans_pinned_urls_and_digests_from_metadata() {
        let plan = plan_from_metadata(
            DEFAULT_ENDPOINT,
            &repository(),
            Some("main"),
            &metadata(0),
            DEFAULT_MAX_DOWNLOAD_BYTES,
        )
        .unwrap();
        assert_eq!(plan.revision, COMMIT);
        assert_eq!(plan.files.len(), 3);
        assert_eq!(
            plan.files[1].url,
            format!("https://huggingface.co/org/model/resolve/{COMMIT}/model.safetensors")
        );
        assert!(matches!(
            plan.files[0].expectation.digest,
            Digest::GitBlobSha1(_)
        ));
        assert!(matches!(
            plan.files[1].expectation.digest,
            Digest::Sha256(_)
        ));
        assert_eq!(plan.total_bytes(), 202 + 30_236_760 + 683_666);
    }

    #[test]
    fn rejects_missing_files_size_limits_and_other_commits() {
        let missing = r#"{"sha": "bf8b056651a2c21b8d2565580b8569da283cab23", "siblings": []}"#;
        let error = plan_from_metadata(DEFAULT_ENDPOINT, &repository(), None, missing, u64::MAX)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnsupportedModelFormat);

        let error = plan_from_metadata(DEFAULT_ENDPOINT, &repository(), None, &metadata(0), 1000)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ModelTooLarge);

        let other = "0".repeat(40);
        let error = plan_from_metadata(
            DEFAULT_ENDPOINT,
            &repository(),
            Some(&other),
            &metadata(0),
            u64::MAX,
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ModelIntegrity);

        let error =
            plan_from_metadata(DEFAULT_ENDPOINT, &repository(), None, "{", u64::MAX).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ModelIntegrity);
    }

    #[test]
    fn validates_endpoints() {
        assert_eq!(
            validate_endpoint("https://hub.internal/").unwrap(),
            "https://hub.internal"
        );
        assert!(validate_endpoint("http://127.0.0.1:8080").is_ok());
        for bad in [
            "ftp://x",
            "https://",
            "https://user:pass@x",
            "https://x?a=1",
            "x",
        ] {
            assert!(validate_endpoint(bad).is_err(), "{bad}");
        }
    }
}
