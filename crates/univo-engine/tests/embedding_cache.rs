use std::sync::{Arc, Mutex};
use univo_core::{FieldPath, Map, Schema, Text, TextMode, Value, prepare};
use univo_embed::{Digest, Embedder, Embeddings, FileExpectation, ModelDescriptor, ModelSource};
use univo_engine::{
    EmbeddingCache, SearchIndex, SearchOptions, embed_records, embed_records_cached,
};

struct Counting {
    model: ModelDescriptor,
    texts: Mutex<Vec<String>>,
    fail: bool,
    batches: Mutex<Vec<usize>>,
}
impl Counting {
    fn new() -> Self {
        Self {
            model: ModelDescriptor {
                source: ModelSource::Local {
                    path: "test".into(),
                    files: vec![FileExpectation {
                        name: "tokenizer.json".into(),
                        size: 1,
                        digest: Digest::Sha256([1; 32]),
                    }],
                },
                dimension: 2,
                backend: "test",
            },
            texts: Mutex::new(Vec::new()),
            fail: false,
            batches: Mutex::new(Vec::new()),
        }
    }
}
impl Embedder for Counting {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.model
    }
    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        self.batches.lock().unwrap().push(texts.len());
        self.texts
            .lock()
            .unwrap()
            .extend(texts.iter().map(|t| (*t).to_owned()));
        if self.fail || texts.contains(&"fail") {
            return Err(univo_core::Error::corrupt_model(
                "test",
                "simulated failure",
            ));
        }
        Embeddings::new(
            "test",
            texts
                .iter()
                .flat_map(|text| {
                    if text.starts_with("unknown") {
                        [0.0, 0.0]
                    } else {
                        [1.0, 0.0]
                    }
                })
                .collect(),
            2,
        )
    }
}
fn schema() -> Schema {
    Schema::builder()
        .text(
            FieldPath::parse("text").unwrap(),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .build()
        .unwrap()
}
fn record(schema: &Schema, text: &str) -> univo_core::PreparedRecord {
    prepare(
        schema,
        &Value::Map(Map::new(vec![("text".into(), Value::Str(text.into()))]).unwrap()),
    )
    .unwrap()
}

#[test]
fn normalized_texts_are_reused_across_add_query_and_indexes() {
    let schema = schema();
    let model = Arc::new(Counting::new());
    let cache = Arc::new(EmbeddingCache::new(model.descriptor().clone(), 8192));
    let mut first = SearchIndex::new(schema.clone(), 100, Some(model.clone()))
        .unwrap()
        .with_embedding_cache(cache.clone())
        .unwrap();
    first
        .add(vec![record(&schema, "STRASSE"), record(&schema, "unknown")])
        .unwrap();
    let options = SearchOptions {
        min_score: 0.8,
        limit: 10,
        max_entries: usize::MAX,
    };
    first.search(&record(&schema, "Straße"), options).unwrap();
    first
        .search_exhaustive(&record(&schema, "unknown"), options)
        .unwrap();
    let mut second = SearchIndex::new(schema.clone(), 100, Some(model.clone()))
        .unwrap()
        .with_embedding_cache(cache.clone())
        .unwrap();
    second
        .add(vec![record(&schema, "strasse"), record(&schema, "unknown")])
        .unwrap();
    assert_eq!(*model.texts.lock().unwrap(), ["strasse", "unknown"]);
    let stats = cache.stats().unwrap();
    assert_eq!((stats.misses, stats.hits, stats.entries), (2, 4, 2));
    assert!(stats.bytes <= stats.max_bytes);
}

#[test]
fn every_retained_byte_is_budgeted_even_after_container_growth_and_eviction() {
    let model = Counting::new();
    let schema = schema();
    for budget in [0, 1, 256, 512, 1024, 4096] {
        let cache = EmbeddingCache::new(model.descriptor().clone(), budget);
        for i in 0..80 {
            let records = vec![record(&schema, &format!("text-{i}"))];
            embed_records_cached(&schema, &records, &model, Some(&cache)).unwrap();
            assert!(cache.stats().unwrap().bytes <= budget);
        }
        let large = record(&schema, &"x".repeat(budget + 1));
        embed_records_cached(&schema, &[large], &model, Some(&cache)).unwrap();
        assert!(cache.stats().unwrap().bytes <= budget);
        if budget < 512 {
            assert_eq!(cache.stats().unwrap().entries, 0);
        } else {
            assert!(cache.stats().unwrap().evictions > 0);
        }
    }
}

#[test]
fn fifo_does_not_refresh_hits_and_evicted_texts_are_reencoded() {
    let model = Counting::new();
    let schema = schema();
    let cache = EmbeddingCache::new(model.descriptor().clone(), 512);
    for text in ["a", "b", "a", "c", "a"] {
        embed_records_cached(&schema, &[record(&schema, text)], &model, Some(&cache)).unwrap();
    }
    assert_eq!(*model.texts.lock().unwrap(), ["a", "b", "c", "a"]);
    assert!(cache.stats().unwrap().evictions >= 1);
}

#[test]
fn complete_model_identity_is_checked_before_even_a_cache_hit() {
    let model = Counting::new();
    let schema = schema();
    let records = vec![record(&schema, "a")];
    let cache = EmbeddingCache::new(model.descriptor().clone(), 4096);
    embed_records_cached(&schema, &records, &model, Some(&cache)).unwrap();
    for variant in 0..4 {
        let mut changed = Counting::new();
        match variant {
            0 => changed.model.dimension = 3,
            1 => changed.model.backend = "other",
            2 => {
                if let ModelSource::Local { files, .. } = &mut changed.model.source {
                    files[0].digest = Digest::Sha256([2; 32]);
                }
            }
            _ => {
                if let ModelSource::Local { path, .. } = &mut changed.model.source {
                    *path = "other".into();
                }
            }
        }
        let before = cache.stats().unwrap();
        let error = embed_records_cached(&schema, &records, &changed, Some(&cache)).unwrap_err();
        assert_eq!(error.code().as_str(), "state.model_mismatch");
        assert_eq!(cache.stats().unwrap(), before);
        assert!(changed.texts.lock().unwrap().is_empty());
    }
}

#[test]
fn failed_encoding_publishes_nothing_and_cache_does_not_change_vectors() {
    let schema = schema();
    let mut model = Counting::new();
    let cache = EmbeddingCache::new(model.descriptor().clone(), 8192);
    let records = vec![
        record(&schema, "a"),
        record(&schema, "unknown"),
        record(&schema, "a"),
    ];
    model.fail = true;
    assert!(embed_records_cached(&schema, &records, &model, Some(&cache)).is_err());
    assert_eq!(cache.stats().unwrap().entries, 0);
    model.fail = false;
    let plain = embed_records(&schema, &records, &model).unwrap();
    let cold = embed_records_cached(&schema, &records, &model, Some(&cache)).unwrap();
    let warm = embed_records_cached(&schema, &records, &model, Some(&cache)).unwrap();
    assert_eq!(plain, cold);
    assert_eq!(cold, warm);
}

#[test]
fn concurrent_warm_readers_reuse_vectors_without_encoding() {
    let schema = schema();
    let records = vec![record(&schema, "same")];
    let model = Counting::new();
    let cache = EmbeddingCache::new(model.descriptor().clone(), 8192);
    let expected = embed_records_cached(&schema, &records, &model, Some(&cache)).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                for _ in 0..40 {
                    let found =
                        embed_records_cached(&schema, &records, &model, Some(&cache)).unwrap();
                    assert_eq!(found, expected);
                }
            });
        }
    });
    assert_eq!(model.texts.lock().unwrap().len(), 1);
    assert_eq!(cache.stats().unwrap().hits, 160);
}

#[test]
fn an_entry_that_cannot_fit_an_empty_cache_preserves_hot_entries() {
    let model = Counting::new();
    let schema = schema();
    let cache = EmbeddingCache::new(model.descriptor().clone(), 512);
    let warm = [record(&schema, "a")];
    embed_records_cached(&schema, &warm, &model, Some(&cache)).unwrap();
    let before = cache.stats().unwrap();
    // Payload exactly fits the budget, but its containers cannot fit.
    let oversized = "x".repeat(440);
    embed_records_cached(
        &schema,
        &[record(&schema, &oversized)],
        &model,
        Some(&cache),
    )
    .unwrap();
    let rejected = cache.stats().unwrap();
    assert_eq!(rejected.entries, before.entries);
    assert_eq!(rejected.bytes, before.bytes);
    assert_eq!(rejected.evictions, before.evictions);
    embed_records_cached(&schema, &warm, &model, Some(&cache)).unwrap();
    assert_eq!(cache.stats().unwrap().hits, before.hits + 1);
    assert_eq!(*model.texts.lock().unwrap(), ["a", &oversized]);
}

#[test]
fn cached_and_uncached_absent_positions_keep_interleaved_rows() {
    let model = Counting::new();
    let schema = schema();
    let cache = EmbeddingCache::new(model.descriptor().clone(), 4096);
    let records: Vec<_> = ["unknown-0", "a", "unknown-1", "b", "unknown-2"]
        .into_iter()
        .map(|text| record(&schema, text))
        .collect();
    let plain = embed_records(&schema, &records, &model).unwrap();
    let cached = embed_records_cached(&schema, &records, &model, Some(&cache)).unwrap();
    assert_eq!(plain, cached);
    for slot in [0, 2, 4] {
        assert!(plain.field(0).unwrap().get(slot).is_none());
    }
    for slot in [1, 3] {
        assert_eq!(
            plain.field(0).unwrap().get(slot),
            Some([1.0, 0.0].as_slice())
        );
    }
}

#[test]
fn an_entry_at_the_empty_cache_boundary_can_replace_an_old_entry() {
    let model = Counting::new();
    let schema = schema();
    let budget = 512;
    let cache = EmbeddingCache::new(model.descriptor().clone(), budget);
    embed_records_cached(&schema, &[record(&schema, "a")], &model, Some(&cache)).unwrap();
    // "a" occupies one text byte; replace it with exactly the available
    // text length, retaining the same vector and container allocation.
    let text = "x".repeat(budget - cache.stats().unwrap().bytes + 1);
    embed_records_cached(&schema, &[record(&schema, &text)], &model, Some(&cache)).unwrap();
    let stats = cache.stats().unwrap();
    assert_eq!(stats.bytes, budget);
    assert_eq!(stats.entries, 1);
    assert_eq!(stats.evictions, 1);
    embed_records_cached(&schema, &[record(&schema, &text)], &model, Some(&cache)).unwrap();
    assert_eq!(cache.stats().unwrap().hits, 1);
}

#[test]
fn encoder_buffers_are_chunked_with_identical_rows_and_warm_reuse() {
    let schema = schema();
    let plain_model = Counting::new();
    let cached_model = Counting::new();
    let cache = EmbeddingCache::new(cached_model.model.clone(), 1 << 20);
    let mut records: Vec<_> = (0..2049)
        .map(|i| {
            record(
                &schema,
                &format!(
                    "{} {i}",
                    if i == 1023 || i == 1024 {
                        "unknown"
                    } else {
                        "known"
                    }
                ),
            )
        })
        .collect();
    records.push(records[0].clone());
    let plain = embed_records(&schema, &records, &plain_model).unwrap();
    let cold = embed_records_cached(&schema, &records, &cached_model, Some(&cache)).unwrap();
    let warm = embed_records_cached(&schema, &records, &cached_model, Some(&cache)).unwrap();
    assert_eq!(plain, cold);
    assert_eq!(cold, warm);
    assert_eq!(*plain_model.batches.lock().unwrap(), [1024, 1024, 1]);
    assert_eq!(*cached_model.batches.lock().unwrap(), [1024, 1024, 1]);
    let field = warm.field(0).unwrap();
    assert!(field.get(1023).is_none());
    assert!(field.get(1024).is_none());
    assert_eq!(field.get(0), field.get(2049));
}

#[test]
fn later_encoding_failure_does_not_partially_add_records() {
    let schema = schema();
    let model = Arc::new(Counting::new());
    let cache = Arc::new(EmbeddingCache::new(model.model.clone(), 1 << 20));
    let mut index = SearchIndex::new(schema.clone(), 3000, Some(model.clone()))
        .unwrap()
        .with_embedding_cache(cache.clone())
        .unwrap();
    let mut records: Vec<_> = (0..2048)
        .map(|i| record(&schema, &format!("known {i}")))
        .collect();
    records.push(record(&schema, "fail"));
    assert!(index.add(records).is_err());
    assert_eq!(index.len(), 0);
    assert_eq!(cache.stats().unwrap().entries, 2048);
}

#[test]
fn real_model_chunked_rows_equal_a_whole_backend_batch() {
    let Some(directory) = std::env::var_os("UNIVO_TEST_MODEL_DIR") else {
        return;
    };
    let model =
        univo_embed::StaticEmbedder::load_directory(std::path::Path::new(&directory)).unwrap();
    let schema = schema();
    let inputs: Vec<_> = (0..2050)
        .map(|i| {
            if i % 341 == 0 {
                String::new()
            } else {
                format!(
                    "local duplicate detection with selected text and semantic evidence item {i}"
                )
            }
        })
        .collect();
    let texts: Vec<_> = inputs.iter().map(String::as_str).collect();
    let direct = model.encode_batch(&texts).unwrap();
    let records: Vec<_> = texts.iter().map(|text| record(&schema, text)).collect();
    let chunked = embed_records(&schema, &records, &model).unwrap();
    let field = chunked.field(0).unwrap();
    for i in 0..texts.len() {
        if direct.absent().binary_search(&i).is_ok() {
            assert!(field.get(i).is_none());
        } else {
            assert_eq!(field.get(i).unwrap(), direct.row(i));
        }
    }
}
