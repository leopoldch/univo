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
        }
    }
}
impl Embedder for Counting {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.model
    }
    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        self.texts
            .lock()
            .unwrap()
            .extend(texts.iter().map(|t| (*t).to_owned()));
        if self.fail {
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
                    if *text == "unknown" {
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
