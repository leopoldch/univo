//! Loading and encoding with the static backend, on the tiny test model.

mod support;

use support::{tiny_model, tiny_model_copy};
use univo_core::ErrorCode;
use univo_embed::{Embedder, ModelSource, StaticEmbedder};

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[test]
fn loads_a_local_model_and_encodes_unit_vectors() {
    let embedder = StaticEmbedder::load_directory(&tiny_model()).unwrap();
    assert_eq!(embedder.descriptor().dimension, 8);
    assert!(matches!(
        embedder.descriptor().source,
        ModelSource::Local { .. }
    ));

    let embeddings = embedder
        .encode_batch(&["Amazon", "amazon online retail", "Société Générale"])
        .unwrap();
    assert_eq!(embeddings.len(), 3);
    for index in 0..3 {
        let norm: f32 = embeddings
            .row(index)
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            .sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "row {index} has norm {norm}");
    }
    assert_eq!(embeddings.absent(), [] as [usize; 0]);
}

#[test]
fn encoding_is_deterministic_and_case_insensitive_for_this_tokenizer() {
    let embedder = StaticEmbedder::load_directory(&tiny_model()).unwrap();
    let first = embedder.encode_batch(&["Amazon", "amazon"]).unwrap();
    let second = embedder.encode_batch(&["Amazon", "amazon"]).unwrap();
    assert_eq!(first, second);
    assert!((cosine(first.row(0), first.row(1)) - 1.0).abs() < 1e-6);
}

#[test]
fn texts_without_known_tokens_are_absent_not_normalized() {
    let embedder = StaticEmbedder::load_directory(&tiny_model()).unwrap();
    let embeddings = embedder.encode_batch(&["amazon", "zzz qqq", ""]).unwrap();
    assert_eq!(embeddings.absent(), [1, 2]);
    assert!(embeddings.row(1).iter().all(|value| *value == 0.0));
}

#[test]
fn long_texts_are_not_truncated() {
    let embedder = StaticEmbedder::load_directory(&tiny_model()).unwrap();
    let mut long = "zzz ".repeat(5000);
    long.push_str("amazon");
    let embeddings = embedder.encode_batch(&[&long, "amazon"]).unwrap();
    // The only known token is at the very end: truncation would lose it.
    assert!((cosine(embeddings.row(0), embeddings.row(1)) - 1.0).abs() < 1e-6);
}

#[test]
fn missing_directories_and_files_are_reported() {
    let error = StaticEmbedder::load_directory(std::path::Path::new("/nonexistent/univo/model"))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelNotAvailable);

    let copy = tiny_model_copy("missing-file");
    std::fs::remove_file(copy.path().join("tokenizer.json")).unwrap();
    let error = StaticEmbedder::load_directory(copy.path()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedModelFormat);
    assert!(error.message().contains("tokenizer.json"), "{error}");
}

#[test]
fn corrupt_files_are_reported_without_panicking() {
    for (file, content) in [
        (
            "model.safetensors",
            b"\x08\x00\x00\x00\x00\x00\x00\x00{}garbage".as_slice(),
        ),
        ("tokenizer.json", b"{not json".as_slice()),
        ("config.json", b"[]".as_slice()),
    ] {
        let copy = tiny_model_copy("corrupt");
        std::fs::write(copy.path().join(file), content).unwrap();
        let result = StaticEmbedder::load_directory(copy.path());
        let code = result.as_ref().err().map(univo_core::Error::code);
        assert!(
            code == Some(ErrorCode::CorruptModel) || (file == "config.json" && result.is_ok()),
            "{file}: {code:?}"
        );
    }
}
