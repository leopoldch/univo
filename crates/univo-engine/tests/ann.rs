use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use univo_core::{
    Conflict, Exact, FieldPath, Map, PreparedRecord, RecordId, Schema, Text, TextMode, Value,
    prepare,
};
use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};
use univo_engine::{AnnOptions, SearchIndex, SearchOptions, snapshot::Snapshot};

struct Model {
    descriptor: ModelDescriptor,
    calls: AtomicUsize,
}
impl Model {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            descriptor: ModelDescriptor {
                source: ModelSource::Local {
                    path: "ann-test".into(),
                    files: vec![],
                },
                dimension: 2,
                backend: "test",
            },
            calls: AtomicUsize::new(0),
        })
    }
}
impl Embedder for Model {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let data = texts
            .iter()
            .flat_map(|text| {
                let number: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
                let angle = f32::from(number % 37) * 0.15;
                [angle.cos(), angle.sin()]
            })
            .collect();
        Embeddings::new("test", data, 2)
    }
}
fn path(text: &str) -> FieldPath {
    FieldPath::parse(text).unwrap()
}
fn schema() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .partition_by(path("tenant"))
        .text(
            path("text"),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .exact(
            path("data"),
            Exact {
                weight: 0.01,
                ..Exact::default()
            },
        )
        .exact(
            path("env"),
            Exact {
                weight: 0.01,
                conflict: Conflict::Reject,
                ..Exact::default()
            },
        )
        .build()
        .unwrap()
}
fn record(
    schema: &Schema,
    id: i64,
    tenant: &str,
    number: u16,
    data: i64,
    env: Option<&str>,
) -> PreparedRecord {
    let mut fields = vec![
        ("id".into(), Value::Int(id)),
        ("tenant".into(), Value::Str(tenant.into())),
        ("text".into(), Value::Str(format!("p {number}"))),
        ("data".into(), Value::Int(data)),
    ];
    if let Some(env) = env {
        fields.push(("env".into(), Value::Str(env.into())));
    }
    prepare(schema, &Value::Map(Map::new(fields).unwrap())).unwrap()
}
fn options() -> SearchOptions {
    SearchOptions {
        min_score: 0.7,
        limit: 100_000,
        max_entries: usize::MAX,
    }
}
fn index() -> (SearchIndex, Arc<Model>) {
    let schema = schema();
    let model = Model::new();
    let mut index = SearchIndex::new(schema.clone(), 10_001, Some(model.clone()))
        .unwrap()
        .with_ann(AnnOptions {
            ef_search: 512,
            max_candidates: 32,
        })
        .unwrap();
    index
        .add(
            (0..10_000)
                .map(|id| {
                    record(
                        &schema,
                        id,
                        "a",
                        u16::try_from(id % 37).unwrap(),
                        id,
                        Some("prod"),
                    )
                })
                .collect(),
        )
        .unwrap();
    (index, model)
}

#[test]
fn ann_is_explicit_scoped_saturated_and_reranked_against_the_oracle() {
    let (mut index, _) = index();
    let schema = index.schema().clone();
    assert_eq!(index.ann_nodes(), 37); // Initial batch slots must not be inserted twice.
    let query = record(&schema, 0, "a", 0, -1, None);
    let found = index.search(&query, options()).unwrap();
    let oracle = index.search_exhaustive(&query, options()).unwrap();
    assert!(found.approximate && found.truncated);
    assert!(!oracle.approximate);
    assert_eq!(found.candidates, 32);
    assert!(oracle.total_matches > found.total_matches);
    for matched in &found.matches {
        assert_ne!(
            index.store().records()[matched.slot].id(),
            Some(&RecordId::Int(0))
        );
        assert!(oracle.matches.contains(matched));
    }
    index
        .add(vec![record(&schema, 10_000, "rare", 0, -1, None)])
        .unwrap();
    let rare = index
        .search(&record(&schema, -1, "rare", 0, -1, None), options())
        .unwrap();
    assert!(!rare.approximate && !rare.truncated);
    assert_eq!(rare.matches.len(), 1);
    assert_eq!(
        index.store().records()[rare.matches[0].slot].id(),
        Some(&RecordId::Int(10_000))
    );
    assert_eq!(
        index
            .search(&record(&schema, -1, "a", 0, -1, Some("test")), options())
            .unwrap()
            .matches,
        []
    );
}

#[test]
fn no_saturation_still_reports_approximation_and_exact_hits_escape_ann_caps() {
    let schema = schema();
    let model = Model::new();
    let mut index = SearchIndex::new(schema.clone(), 10_000, Some(model))
        .unwrap()
        .with_ann(AnnOptions {
            ef_search: 512,
            max_candidates: 1,
        })
        .unwrap();
    index
        .add(
            (0..10_000)
                .map(|id| record(&schema, id, "a", 0, 0, Some("prod")))
                .collect(),
        )
        .unwrap();
    let query = record(&schema, -1, "a", 0, 0, Some("prod"));
    let found = index.search(&query, options()).unwrap();
    assert_eq!(found.lanes.exact, 10_000);
    assert_eq!(found.total_matches, 10_000);
    assert!(found.approximate && found.truncated);
    index
        .remove(&(0..9999).map(RecordId::Int).collect::<Vec<_>>())
        .unwrap();
    let found = index.search(&query, options()).unwrap();
    assert!(found.approximate && !found.truncated);
    assert_eq!(found.total_matches, 1);
}

#[test]
fn mutations_mask_old_versions_and_compaction_rebuilds_without_inference() {
    let (mut index, model) = index();
    let schema = index.schema().clone();
    index
        .upsert(
            (0..5001)
                .map(|id| {
                    record(
                        &schema,
                        id,
                        "b",
                        u16::try_from(id % 37).unwrap(),
                        id,
                        Some("prod"),
                    )
                })
                .collect(),
        )
        .unwrap();
    let query = record(&schema, -1, "a", 0, -1, None);
    for matched in index.search(&query, options()).unwrap().matches {
        let Some(RecordId::Int(id)) = index.store().records()[matched.slot].id() else {
            panic!("integer ID")
        };
        assert!(*id >= 5001);
    }
    let calls = model.calls.load(Ordering::Relaxed);
    index
        .upsert(vec![record(&schema, 9999, "a", 9, 9999, Some("prod"))])
        .unwrap();
    // ID 9999 originally had remainder 9; compaction and this replacement reuse its vector.
    assert_eq!(model.calls.load(Ordering::Relaxed), calls);
    assert_eq!(index.ann_nodes(), 0); // Two small partitions fall back to dense routing.
    assert!(!index.search(&query, options()).unwrap().approximate);
    index.remove(&[RecordId::Int(5001)]).unwrap();
    assert!(!index.store().contains_id(&RecordId::Int(5001)));
}

#[test]
fn snapshots_record_ann_identity_rebuild_without_encoding_and_reject_mismatch() {
    let (index, model) = index();
    let schema = index.schema().clone();
    let directory = std::env::temp_dir().join(format!("univo-ann-snapshot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    index
        .save(&directory, 1, serde_json::Value::Null, 16 << 20)
        .unwrap();
    model.calls.store(0, Ordering::Relaxed);
    let empty = || SearchIndex::new(schema.clone(), 10_001, Some(model.clone())).unwrap();
    assert!(
        Snapshot::open(&directory, 16 << 20)
            .unwrap()
            .restore(empty())
            .is_err()
    );
    let restored = Snapshot::open(&directory, 16 << 20)
        .unwrap()
        .restore(
            empty()
                .with_ann(AnnOptions {
                    ef_search: 512,
                    max_candidates: 32,
                })
                .unwrap(),
        )
        .unwrap();
    assert_eq!(model.calls.load(Ordering::Relaxed), 0);
    assert_eq!(restored.ann_nodes(), index.ann_nodes());
    let query = record(&schema, -1, "a", 0, -1, None);
    assert_eq!(
        restored.search(&query, options()).unwrap(),
        index.search(&query, options()).unwrap()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn ann_options_are_validated_and_default_indexes_remain_exact() {
    for options in [
        AnnOptions {
            ef_search: 0,
            max_candidates: 1,
        },
        AnnOptions {
            ef_search: 8193,
            max_candidates: 1,
        },
        AnnOptions {
            ef_search: 1,
            max_candidates: 0,
        },
        AnnOptions {
            ef_search: 1,
            max_candidates: 100_001,
        },
    ] {
        assert!(options.validate().is_err());
    }
    let schema = schema();
    let mut index = SearchIndex::new(schema.clone(), 1, Some(Model::new())).unwrap();
    index
        .add(vec![record(&schema, 0, "a", 0, 0, None)])
        .unwrap();
    assert_eq!(index.ann_bytes(), 0);
    assert!(
        !index
            .search(&record(&schema, -1, "a", 0, 0, None), options())
            .unwrap()
            .approximate
    );
    assert!(index.with_ann(AnnOptions::default()).is_err());
}
