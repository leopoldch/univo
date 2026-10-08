//! The download flow a binding drives: plan, stage, write, publish, find,
//! load. Files are copied from the tiny test model instead of downloaded.

mod support;

use std::time::Duration;

use sha1::Digest as _;
use support::{TempDir, tiny_model};
use univo_core::ErrorCode;
use univo_embed::hub::plan_from_metadata;
use univo_embed::{
    Cache, DEFAULT_ENDPOINT, DownloadPlan, Embedder, MODEL_FILES, ModelSource, Repository, Stage,
    Staging, StaticEmbedder,
};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn repository() -> Repository {
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

/// Hub metadata for the tiny model: the weights as an LFS file with a
/// SHA-256, the JSON files as Git blobs with a SHA-1, as the hub publishes.
fn metadata() -> String {
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

fn plan() -> DownloadPlan {
    plan_at(Some("main"))
}

fn plan_at(revision: Option<&str>) -> DownloadPlan {
    plan_from_metadata(
        DEFAULT_ENDPOINT,
        &repository(),
        revision,
        &metadata(),
        u64::MAX,
    )
    .unwrap()
}

fn staging(cache: &Cache, plan: &DownloadPlan) -> Staging {
    match cache.stage(plan).unwrap() {
        Stage::Download(staging) => staging,
        Stage::Published(_) => panic!("the model is already published"),
    }
}

fn download(cache: &Cache, plan: &DownloadPlan) -> univo_core::Result<univo_embed::CachedModel> {
    let staging = staging(cache, plan);
    for file in &plan.files {
        std::fs::copy(
            tiny_model().join(&file.expectation.name),
            staging.file_path(&file.expectation.name),
        )
        .unwrap();
    }
    cache.publish(staging)
}

/// Whether no download of the test repository is in progress or left over.
fn no_partial_download(cache: &Cache) -> bool {
    let directory = cache.root().join("models/univo-test/tiny");
    std::fs::read_dir(directory).map_or(true, |entries| {
        entries.into_iter().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".partial")
        })
    })
}

#[test]
fn a_published_model_is_found_by_reference_and_loaded() {
    let root = TempDir::new("publish");
    let cache = Cache::new(root.path());
    let plan = plan();
    assert!(cache.find(&repository(), None).unwrap().is_none());

    let published = download(&cache, &plan).unwrap();
    assert_eq!(published.revision, COMMIT);
    assert!(no_partial_download(&cache));

    let by_branch = cache.find(&repository(), Some("main")).unwrap().unwrap();
    let by_default = cache.find(&repository(), None).unwrap().unwrap();
    let by_commit = cache
        .find(&repository(), Some(&COMMIT.to_ascii_uppercase()))
        .unwrap()
        .unwrap();
    assert_eq!(by_branch, published);
    assert_eq!(by_default, published);
    assert_eq!(by_commit, published);
    assert!(cache.find(&repository(), Some("v2")).unwrap().is_none());

    let embedder = StaticEmbedder::load_cached(&by_branch).unwrap();
    let ModelSource::Hub {
        revision, files, ..
    } = &embedder.descriptor().source
    else {
        panic!("a cached model is a hub model");
    };
    assert_eq!(revision, COMMIT);
    assert_eq!(files.len(), MODEL_FILES.len());
    assert_eq!(
        embedder.descriptor().name(),
        format!("univo-test/tiny@{COMMIT}")
    );
    assert_eq!(embedder.encode_batch(&["amazon"]).unwrap().dimension(), 8);

    // A second download of the same commit finds it published.
    assert!(matches!(cache.stage(&plan).unwrap(), Stage::Published(model) if model == published));
}

#[test]
fn a_corrupted_download_is_rejected_and_nothing_is_published() {
    let root = TempDir::new("corrupt");
    let cache = Cache::new(root.path());
    let plan = plan();
    let staging = staging(&cache, &plan);
    for file in MODEL_FILES {
        let mut content = std::fs::read(tiny_model().join(file)).unwrap();
        if file == "model.safetensors" {
            let last = content.len() - 1;
            content[last] ^= 1;
        }
        std::fs::write(staging.file_path(file), content).unwrap();
    }
    let error = cache.publish(staging).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelIntegrity);
    assert!(error.message().contains("model.safetensors"), "{error}");

    assert!(cache.find(&repository(), None).unwrap().is_none());
    assert!(no_partial_download(&cache));
    // The lock was released: a retry succeeds.
    download(&cache, &plan).unwrap();
}

#[test]
fn a_missing_file_is_rejected() {
    let root = TempDir::new("missing");
    let cache = Cache::new(root.path());
    let staging = staging(&cache, &plan());
    std::fs::copy(
        tiny_model().join("config.json"),
        staging.file_path("config.json"),
    )
    .unwrap();
    let error = cache.publish(staging).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelIntegrity);
    assert!(cache.find(&repository(), None).unwrap().is_none());
}

#[test]
fn a_held_lock_times_out_with_a_clear_error() {
    let root = TempDir::new("locked");
    let cache = Cache::new(root.path()).with_lock_timeout(Duration::from_millis(250));
    let plan = plan();
    let held = staging(&cache, &plan);

    let error = cache.stage(&plan).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelCacheLocked);
    assert!(error.message().contains(".lock"), "{error}");

    drop(held);
    assert!(matches!(cache.stage(&plan).unwrap(), Stage::Download(_)));
}

#[test]
fn a_cached_file_that_changed_size_is_reported() {
    let root = TempDir::new("tamper");
    let cache = Cache::new(root.path());
    let published = download(&cache, &plan()).unwrap();
    std::fs::write(published.path.join("tokenizer.json"), b"{}").unwrap();

    let error = cache.find(&repository(), None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelIntegrity);
    assert!(error.message().contains("tokenizer.json"), "{error}");
}

#[test]
fn a_malformed_manifest_is_reported() {
    let root = TempDir::new("manifest");
    let cache = Cache::new(root.path());
    let published = download(&cache, &plan()).unwrap();
    std::fs::write(published.path.join(univo_embed::cache::MANIFEST), b"{").unwrap();

    let error = cache.find(&repository(), None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelIntegrity);
}

#[test]
fn the_pinned_default_model_loads_when_present_locally() {
    // Runs only where the default model was downloaded, for example by
    // `huggingface-cli download minishlab/potion-base-8M`; the files are
    // verified against the pinned checksums before loading.
    let Some(directory) = std::env::var_os("UNIVO_TEST_MODEL_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    let pinned = univo_embed::registry::default_model();
    for file in &pinned.files {
        univo_embed::files::verify_file("default", &directory.join(&file.name), file).unwrap();
    }
    let embedder = StaticEmbedder::load_directory(&directory).unwrap();
    assert_eq!(embedder.descriptor().dimension, 256);
    let embeddings = embedder
        .encode_batch(&["Amazon", "Amazon.com Inc.", "Banana"])
        .unwrap();
    let cosine = |a: usize, b: usize| -> f32 {
        embeddings
            .row(a)
            .iter()
            .zip(embeddings.row(b))
            .map(|(x, y)| x * y)
            .sum()
    };
    assert!(cosine(0, 1) > cosine(0, 2));
}

/// Sets the mode of `directory` and of every directory below it.
#[cfg(unix)]
fn set_directory_modes(directory: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;

    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            set_directory_modes(&path, mode);
        }
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(unix)]
#[test]
fn a_read_only_cache_serves_published_models() {
    let root = TempDir::new("read-only");
    let cache = Cache::new(root.path());
    download(&cache, &plan()).unwrap();
    set_directory_modes(root.path(), 0o555);
    let found = cache.find(&repository(), None);
    let loaded = found
        .as_ref()
        .map(|model| model.as_ref().map(StaticEmbedder::load_cached));
    set_directory_modes(root.path(), 0o755);

    assert!(matches!(loaded, Ok(Some(Ok(_)))), "{loaded:?}");
}

#[test]
fn a_cached_file_changed_in_place_is_rejected_at_load() {
    let root = TempDir::new("in-place");
    let cache = Cache::new(root.path());
    let published = download(&cache, &plan()).unwrap();
    let path = published.path.join("tokenizer.json");
    let original = std::fs::read(&path).unwrap();
    let edited =
        String::from_utf8(original.clone())
            .unwrap()
            .replacen("\"amazon\"", "\"amazom\"", 1);
    assert_eq!(edited.len(), original.len());
    std::fs::write(&path, edited).unwrap();

    let found = cache.find(&repository(), None).unwrap().unwrap();
    let error = StaticEmbedder::load_cached(&found).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelIntegrity);
    assert!(error.message().contains("tokenizer.json"), "{error}");

    // A file that no longer parses is reported as changed, not as corrupt.
    let mut unreadable = original;
    unreadable[0] = b'x';
    std::fs::write(&path, unreadable).unwrap();
    let error = StaticEmbedder::load_cached(&found).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelIntegrity);
}

#[test]
fn a_model_downloaded_without_a_revision_is_found_offline() {
    let root = TempDir::new("no-revision");
    let cache = Cache::new(root.path());
    download(&cache, &plan_at(None)).unwrap();
    assert!(cache.find(&repository(), None).unwrap().is_some());
}

#[test]
fn a_new_reference_to_a_published_commit_is_recorded() {
    let root = TempDir::new("new-reference");
    let cache = Cache::new(root.path());
    download(&cache, &plan()).unwrap();
    let stage = cache.stage(&plan_at(Some("v1"))).unwrap();
    assert!(matches!(stage, Stage::Published(_)));
    let found = cache.find(&repository(), Some("v1")).unwrap();
    assert_eq!(found.map(|model| model.revision), Some(COMMIT.to_owned()));
}

#[test]
fn an_abandoned_download_leaves_no_files() {
    let root = TempDir::new("abandoned");
    let cache = Cache::new(root.path());
    let staging = staging(&cache, &plan());
    let partial = staging.file_path("model.safetensors");
    std::fs::write(&partial, vec![0u8; 4096]).unwrap();
    drop(staging);
    assert!(!partial.exists());
    assert!(no_partial_download(&cache));
}

#[test]
fn the_leftovers_of_a_dead_download_are_removed_by_the_next_one() {
    let root = TempDir::new("leftover");
    let cache = Cache::new(root.path());
    let leftover = root
        .path()
        .join(format!("models/univo-test/tiny/{COMMIT}.partial"));
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(leftover.join("model.safetensors"), vec![0u8; 4096]).unwrap();

    let staging = staging(&cache, &plan());
    assert!(!staging.file_path("model.safetensors").exists());
    drop(staging);
    assert!(no_partial_download(&cache));
}
