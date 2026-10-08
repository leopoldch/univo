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

/// Rewrites the tokenizer of a copy of the tiny model.
fn edit_tokenizer(directory: &std::path::Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = directory.join("tokenizer.json");
    let mut tokenizer: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    edit(&mut tokenizer);
    std::fs::write(&path, serde_json::to_vec(&tokenizer).unwrap()).unwrap();
}

#[test]
fn truncation_configured_in_the_tokenizer_is_ignored() {
    let copy = tiny_model_copy("truncation");
    edit_tokenizer(copy.path(), |tokenizer| {
        tokenizer["truncation"] = serde_json::json!({
            "direction": "Right", "max_length": 1, "strategy": "LongestFirst", "stride": 0
        });
    });
    let embedder = StaticEmbedder::load_directory(copy.path()).unwrap();
    let embeddings = embedder.encode_batch(&["zzz amazon", "amazon"]).unwrap();
    assert_eq!(embeddings.absent(), [] as [usize; 0]);
    assert_eq!(embeddings.row(0), embeddings.row(1));
}

#[test]
fn padding_configured_in_the_tokenizer_does_not_depend_on_the_batch() {
    let copy = tiny_model_copy("padding");
    edit_tokenizer(copy.path(), |tokenizer| {
        tokenizer["padding"] = serde_json::json!({
            "strategy": "BatchLongest", "direction": "Right", "pad_to_multiple_of": null,
            "pad_id": 1, "pad_type_id": 0, "pad_token": "amazon"
        });
    });
    let embedder = StaticEmbedder::load_directory(copy.path()).unwrap();
    let alone = embedder.encode_batch(&["online"]).unwrap();
    let batched = embedder
        .encode_batch(&["online", "online retail cloud payments"])
        .unwrap();
    assert_eq!(alone.row(0), batched.row(0));
}

#[test]
fn local_models_are_identified_by_their_content() {
    let copy = tiny_model_copy("identity");
    let before = StaticEmbedder::load_directory(copy.path()).unwrap();
    edit_tokenizer(copy.path(), |tokenizer| {
        tokenizer["model"]["vocab"]["amazon"] = serde_json::json!(3);
        tokenizer["model"]["vocab"]["microsoft"] = serde_json::json!(1);
    });
    let after = StaticEmbedder::load_directory(copy.path()).unwrap();
    assert_ne!(
        before.encode_batch(&["amazon"]).unwrap(),
        after.encode_batch(&["amazon"]).unwrap()
    );
    assert_ne!(before.descriptor(), after.descriptor());
}

#[test]
fn large_batches_keep_their_order_and_absent_positions() {
    let embedder = StaticEmbedder::load_directory(&tiny_model()).unwrap();
    let texts: Vec<&str> = (0..10_000)
        .map(|index| match index % 3 {
            0 => "amazon",
            1 => "zzz",
            _ => "online retail",
        })
        .collect();
    let embeddings = embedder.encode_batch(&texts).unwrap();
    let expected_absent: Vec<usize> = (0..10_000).filter(|index| index % 3 == 1).collect();
    assert_eq!(embeddings.absent(), expected_absent);
    let single = embedder.encode_batch(&["online retail"]).unwrap();
    assert_eq!(embeddings.row(9_998), single.row(0));
}
