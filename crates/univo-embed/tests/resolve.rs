//! Resolution through the cache and a fake transport that serves the tiny
//! model as a hub would, and records every request.

mod support;

use std::cell::RefCell;
use std::path::Path;

use support::{COMMIT, TempDir, tiny_metadata, tiny_model};
use univo_core::ErrorCode;
use univo_embed::{
    Cache, Embedder, ModelSpec, ResolveOptions, Resolved, Transport, TransportError, resolve,
};

#[derive(Default)]
struct FakeHub {
    requests: RefCell<Vec<String>>,
    /// The response of the metadata request, when not the tiny model's.
    metadata: Option<Result<String, TransportError>>,
    /// A file whose download fails, or whose last byte is flipped.
    failing: Option<&'static str>,
    corrupting: Option<&'static str>,
}

impl FakeHub {
    fn requests(&self) -> Vec<String> {
        self.requests.borrow().clone()
    }
}

impl Transport for FakeHub {
    fn fetch(&self, url: &str, destination: &Path, max_bytes: u64) -> Result<(), TransportError> {
        self.requests.borrow_mut().push(url.to_owned());
        let file = url.rsplit('/').next().unwrap();
        if self.failing == Some(file) {
            return Err(TransportError {
                status: None,
                reason: "connection reset".into(),
            });
        }
        let mut content = std::fs::read(tiny_model().join(file)).unwrap();
        assert_eq!(content.len() as u64, max_bytes);
        if self.corrupting == Some(file) {
            *content.last_mut().unwrap() ^= 1;
        }
        std::fs::write(destination, content).unwrap();
        Ok(())
    }

    fn fetch_text(&self, url: &str, _max_bytes: u64) -> Result<String, TransportError> {
        self.requests.borrow_mut().push(url.to_owned());
        self.metadata.clone().unwrap_or_else(|| Ok(tiny_metadata()))
    }
}

fn online() -> ResolveOptions {
    ResolveOptions {
        endpoint: "http://127.0.0.1:9".into(),
        ..ResolveOptions::default()
    }
}

fn offline() -> ResolveOptions {
    ResolveOptions {
        offline: true,
        ..online()
    }
}

fn tiny() -> ModelSpec {
    ModelSpec::parse("univo-test/tiny").unwrap()
}

#[test]
fn a_hub_model_is_downloaded_once_then_served_from_the_cache() {
    let root = TempDir::new("resolve-once");
    let cache = Cache::new(root.path());
    let hub = FakeHub::default();

    let resolved = resolve(&tiny(), &cache, &online(), &hub).unwrap();
    assert_eq!(
        hub.requests(),
        [
            "http://127.0.0.1:9/api/models/univo-test/tiny/revision/main?blobs=true".to_owned(),
            format!("http://127.0.0.1:9/univo-test/tiny/resolve/{COMMIT}/config.json"),
            format!("http://127.0.0.1:9/univo-test/tiny/resolve/{COMMIT}/model.safetensors"),
            format!("http://127.0.0.1:9/univo-test/tiny/resolve/{COMMIT}/tokenizer.json"),
        ]
    );
    let embedder = resolved.load().unwrap();
    assert_eq!(embedder.encode_batch(&["amazon"]).unwrap().dimension(), 8);

    let silent = FakeHub::default();
    for options in [online(), offline()] {
        assert_eq!(
            resolve(&tiny(), &cache, &options, &silent).unwrap(),
            resolved
        );
        let pinned = ModelSpec::parse(&format!("univo-test/tiny@{COMMIT}")).unwrap();
        assert_eq!(
            resolve(&pinned, &cache, &options, &silent).unwrap(),
            resolved
        );
    }
    assert_eq!(silent.requests(), [] as [String; 0]);
}

#[test]
fn offline_resolution_never_calls_the_transport() {
    let root = TempDir::new("resolve-offline");
    let cache = Cache::new(root.path());
    let hub = FakeHub::default();
    for spec in [ModelSpec::Default, tiny()] {
        let error = resolve(&spec, &cache, &offline(), &hub).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ModelNotAvailable);
        assert!(error.message().contains("offline"), "{error}");
        assert!(
            error.message().contains(&root.path().display().to_string()),
            "{error}"
        );
    }
    assert_eq!(hub.requests(), [] as [String; 0]);
}

#[test]
fn local_directories_are_used_as_they_are() {
    let root = TempDir::new("resolve-local");
    let hub = FakeHub::default();
    let spec = ModelSpec::Local(tiny_model());
    let resolved = resolve(&spec, &Cache::new(root.path()), &online(), &hub).unwrap();
    assert_eq!(resolved, Resolved::Local(tiny_model()));
    assert_eq!(hub.requests(), [] as [String; 0]);
}

#[test]
fn a_failed_download_publishes_nothing() {
    let root = TempDir::new("resolve-failed");
    let cache = Cache::new(root.path());
    let hub = FakeHub {
        failing: Some("model.safetensors"),
        ..FakeHub::default()
    };
    let error = resolve(&tiny(), &cache, &online(), &hub).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelDownloadFailed);
    assert!(error.message().contains("model.safetensors"), "{error}");
    assert!(error.message().contains("connection reset"), "{error}");
    assert!(resolve(&tiny(), &cache, &offline(), &hub).is_err());
}

#[test]
fn an_altered_download_is_rejected() {
    let root = TempDir::new("resolve-altered");
    let cache = Cache::new(root.path());
    let hub = FakeHub {
        corrupting: Some("tokenizer.json"),
        ..FakeHub::default()
    };
    let error = resolve(&tiny(), &cache, &online(), &hub).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelIntegrity);
    assert!(resolve(&tiny(), &cache, &offline(), &hub).is_err());
}

#[test]
fn hub_answers_are_reported() {
    let root = TempDir::new("resolve-answers");
    let cache = Cache::new(root.path());
    for (answer, code) in [
        (
            Err(TransportError {
                status: Some(404),
                reason: "HTTP 404".into(),
            }),
            ErrorCode::ModelNotAvailable,
        ),
        (
            Err(TransportError {
                status: Some(503),
                reason: "HTTP 503".into(),
            }),
            ErrorCode::ModelDownloadFailed,
        ),
        (Ok("{}".to_owned()), ErrorCode::ModelIntegrity),
        (
            Ok(format!(r#"{{"sha": "{COMMIT}", "siblings": []}}"#)),
            ErrorCode::UnsupportedModelFormat,
        ),
    ] {
        let hub = FakeHub {
            metadata: Some(answer),
            ..FakeHub::default()
        };
        let error = resolve(&tiny(), &cache, &online(), &hub).unwrap_err();
        assert_eq!(error.code(), code, "{error}");
        assert_eq!(hub.requests().len(), 1);
    }
}

#[test]
fn oversized_models_and_invalid_endpoints_are_refused_before_downloading() {
    let root = TempDir::new("resolve-limits");
    let cache = Cache::new(root.path());
    let hub = FakeHub::default();
    let small = ResolveOptions {
        max_download_bytes: 100,
        ..online()
    };
    let error = resolve(&tiny(), &cache, &small, &hub).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelTooLarge);
    assert_eq!(hub.requests().len(), 1);

    let error = resolve(&ModelSpec::Default, &cache, &small, &hub).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelTooLarge);
    assert_eq!(hub.requests().len(), 1);

    let hub = FakeHub::default();
    let invalid = ResolveOptions {
        endpoint: "ftp://hub".into(),
        ..online()
    };
    let error = resolve(&tiny(), &cache, &invalid, &hub).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidModelSpec);
    assert_eq!(hub.requests(), [] as [String; 0]);
}

/// Serves the tiny model's files under any URL.
struct Impostor;
impl Transport for Impostor {
    fn fetch(&self, url: &str, destination: &Path, _: u64) -> Result<(), TransportError> {
        assert!(url.starts_with("http://127.0.0.1:9/minishlab/potion-base-8M/resolve/"));
        let file = url.rsplit('/').next().unwrap();
        std::fs::copy(tiny_model().join(file), destination).unwrap();
        Ok(())
    }
    fn fetch_text(&self, _: &str, _: u64) -> Result<String, TransportError> {
        panic!("the default model needs no metadata");
    }
}

#[test]
fn the_default_model_is_verified_against_the_release_checksums() {
    // Serving the tiny model's files under the default model's URLs: the
    // pinned checksums reject them.
    let root = TempDir::new("resolve-default");
    let cache = Cache::new(root.path());
    let error = resolve(&ModelSpec::Default, &cache, &online(), &Impostor).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelIntegrity);
}
