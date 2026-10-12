use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use univo_core::{
    Conflict, Exact, FieldPath, Map, PreparedRecord, RecordId, Schema, Text, TextMode, Value,
    prepare,
};
use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};
use univo_engine::snapshot::Snapshot;
use univo_engine::{SearchIndex, SearchOptions};

struct Counting {
    descriptor: ModelDescriptor,
    texts: Mutex<Vec<String>>,
}
impl Counting {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            descriptor: ModelDescriptor {
                source: ModelSource::Local {
                    path: "test".into(),
                    files: vec![],
                },
                dimension: 2,
                backend: "test",
            },
            texts: Mutex::new(vec![]),
        })
    }
}
impl Embedder for Counting {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        self.texts
            .lock()
            .unwrap()
            .extend(texts.iter().map(|text| (*text).to_owned()));
        if texts.contains(&"fail") {
            return Err(univo_core::Error::corrupt_model("test", "injected failure"));
        }
        Embeddings::new(
            "test",
            texts
                .iter()
                .flat_map(|text| match *text {
                    "unknown" => [0.0, 0.0],
                    "world" => [0.0, 1.0],
                    _ => [1.0, 0.0],
                })
                .collect(),
            2,
        )
    }
}
fn path(text: &str) -> FieldPath {
    FieldPath::parse(text).unwrap()
}
fn schema() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .partition_by(path("tenant"))
        .text(path("name"), Text::default())
        .text(
            path("description"),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .text(
            path("other"),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .exact(
            path("env"),
            Exact {
                conflict: Conflict::Reject,
                ..Exact::default()
            },
        )
        .build()
        .unwrap()
}
fn input(id: i64, tenant: &str, name: &str, description: &str, env: &str) -> Value {
    Value::Map(
        Map::new(vec![
            ("id".into(), Value::Int(id)),
            ("tenant".into(), Value::Str(tenant.into())),
            ("name".into(), Value::Str(name.into())),
            ("description".into(), Value::Str(description.into())),
            ("other".into(), Value::Str("unknown".into())),
            ("env".into(), Value::Str(env.into())),
        ])
        .unwrap(),
    )
}
fn record(
    schema: &Schema,
    id: i64,
    tenant: &str,
    name: &str,
    description: &str,
    env: &str,
) -> PreparedRecord {
    prepare(schema, &input(id, tenant, name, description, env)).unwrap()
}
fn options(score: f32) -> SearchOptions {
    SearchOptions {
        min_score: score,
        limit: 1000,
        max_entries: usize::MAX,
    }
}
fn hits(
    index: &SearchIndex,
    query: &PreparedRecord,
    score: f32,
    exhaustive: bool,
) -> Vec<(RecordId, univo_core::Comparison)> {
    let result = if exhaustive {
        index.search_exhaustive(query, options(score))
    } else {
        index.search(query, options(score))
    }
    .unwrap();
    result
        .matches
        .into_iter()
        .map(|found| {
            (
                index.store().records()[found.slot].id().unwrap().clone(),
                found.comparison,
            )
        })
        .collect()
}

#[test]
fn unchanged_fields_and_absent_vectors_are_reused_without_a_cache() {
    let schema = schema();
    let model = Counting::new();
    let mut index = SearchIndex::new(schema.clone(), 2, Some(model.clone())).unwrap();
    index
        .add(vec![record(&schema, 1, "a", "old", "hello", "prod")])
        .unwrap();
    model.texts.lock().unwrap().clear();
    index
        .upsert(vec![record(&schema, 1, "b", "new", "HELLO", "test")])
        .unwrap();
    assert!(model.texts.lock().unwrap().is_empty());
    index
        .upsert(vec![record(&schema, 1, "b", "new", "world", "test")])
        .unwrap();
    assert_eq!(*model.texts.lock().unwrap(), ["world"]);
    assert_eq!(
        hits(
            &index,
            &record(&schema, 9, "a", "old", "hello", "prod"),
            0.1,
            false
        ),
        []
    );
    let query = record(&schema, 9, "b", "new", "world", "test");
    assert_eq!(hits(&index, &query, 1.0, false)[0].0, RecordId::Int(1));
    assert_eq!(index.len(), 1);
}

#[test]
fn preparation_capacity_and_inference_failures_do_not_publish_any_record() {
    let schema = schema();
    let model = Counting::new();
    let mut index = SearchIndex::new(schema.clone(), 1, Some(model.clone())).unwrap();
    let old = record(&schema, 1, "a", "old", "hello", "prod");
    index.add(vec![old.clone()]).unwrap();
    model.texts.lock().unwrap().clear();
    assert!(index.upsert(vec![old.clone(), old.clone()]).is_err());
    assert!(
        index
            .upsert(vec![record(&schema, 2, "a", "new", "world", "prod")])
            .is_err()
    );
    assert!(model.texts.lock().unwrap().is_empty());
    assert!(
        index
            .upsert(vec![record(&schema, 1, "b", "new", "fail", "test")])
            .is_err()
    );
    assert_eq!(index.store().records(), [old]);
    assert_eq!(index.len(), 1);
}

#[test]
fn mixed_mutations_match_a_fresh_exhaustive_index_and_compact_without_encoding() {
    let schema = schema();
    let model = Counting::new();
    let mut index = SearchIndex::new(schema.clone(), 32, Some(model.clone())).unwrap();
    let mut expected = BTreeMap::new();
    for step in 0..320 {
        let id = step % 16;
        if step % 5 == 0 {
            let missing = index
                .remove(&[RecordId::Int(id), RecordId::Int(id)])
                .unwrap();
            let existed = expected.remove(&id).is_some();
            assert_eq!(missing.len(), if existed { 0 } else { 2 });
        } else {
            let next = record(
                &schema,
                id,
                if step % 3 == 0 { "b" } else { "a" },
                if step % 2 == 0 { "amazon" } else { "amason" },
                if step % 4 == 0 { "world" } else { "hello" },
                if step % 7 == 0 { "test" } else { "prod" },
            );
            index.upsert(vec![next.clone()]).unwrap();
            expected.insert(id, next);
        }
        assert_eq!(index.len(), expected.len());
        assert!(index.store().len() <= 64);
        if step % 11 != 0 {
            continue;
        }
        let mut fresh = SearchIndex::new(schema.clone(), 32, Some(Counting::new())).unwrap();
        fresh.add(expected.values().cloned().collect()).unwrap();
        for tenant in ["a", "b"] {
            for score in [0.1, 0.7, 1.0] {
                let query = record(&schema, 100, tenant, "amazon", "hello", "prod");
                assert_eq!(
                    hits(&index, &query, score, false),
                    hits(&fresh, &query, score, true)
                );
            }
        }
        let pairs = index.find(options(0.7), false, 1000, 10000).unwrap();
        let oracle = index.find(options(0.7), true, 1000, 10000).unwrap();
        assert_eq!(pairs.matches, oracle.matches);
    }
    model.texts.lock().unwrap().clear();
    let same: Vec<_> = expected.values().cloned().collect();
    for _ in 0..20 {
        index.upsert(same.clone()).unwrap();
    }
    assert!(model.texts.lock().unwrap().is_empty());
}

#[test]
fn snapshots_keep_only_active_records_and_vectors_and_restore_without_inference() {
    let schema = schema();
    let model = Counting::new();
    let mut index = SearchIndex::new(schema.clone(), 3, Some(model.clone())).unwrap();
    index
        .add(vec![
            record(&schema, 1, "a", "DELETED-TEXT", "hello", "prod"),
            record(&schema, 2, "a", "old", "world", "prod"),
        ])
        .unwrap();
    index.remove(&[RecordId::Int(1)]).unwrap();
    index
        .upsert(vec![record(&schema, 2, "b", "new", "hello", "test")])
        .unwrap();
    let directory = std::env::temp_dir().join(format!("univo-mutations-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    index
        .save(&directory, 3, serde_json::Value::Null, 1 << 20)
        .unwrap();
    model.texts.lock().unwrap().clear();
    let snapshot = Snapshot::open(&directory, 1 << 20).unwrap();
    assert_eq!(snapshot.generation(), 3);
    let mut restored = snapshot
        .restore(SearchIndex::new(schema.clone(), 3, Some(model.clone())).unwrap())
        .unwrap();
    assert!(model.texts.lock().unwrap().is_empty());
    assert_eq!(restored.len(), 1);
    assert_eq!(restored.store().len(), 1);
    for entry in std::fs::read_dir(&directory).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            assert!(
                !String::from_utf8_lossy(&std::fs::read(entry.path().join("records.bin")).unwrap())
                    .contains("deleted-text")
            );
        }
    }
    restored
        .upsert(vec![record(&schema, 2, "b", "latest", "HELLO", "test")])
        .unwrap();
    assert!(model.texts.lock().unwrap().is_empty());
    let query = record(&schema, 99, "b", "latest", "hello", "test");
    assert_eq!(hits(&restored, &query, 1.0, false)[0].0, RecordId::Int(2));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mutation_requires_external_ids_and_deleted_slots_cannot_be_queried() {
    let mut strings = SearchIndex::new(Schema::strings(), 2, None).unwrap();
    assert!(strings.upsert(vec![]).is_err());
    assert!(strings.remove(&[]).is_err());
    let schema = schema();
    let mut index = SearchIndex::new(schema.clone(), 2, Some(Counting::new())).unwrap();
    let slots = index
        .add(vec![record(&schema, 1, "a", "name", "hello", "prod")])
        .unwrap();
    index.remove(&[RecordId::Int(1)]).unwrap();
    assert!(index.search_slot(slots.start, options(0.1)).is_err());
    assert_eq!(index.find(options(0.1), false, 0, 0).unwrap().matches, []);
    assert_eq!(index.len(), 0);
    index
        .add(vec![record(&schema, 1, "a", "new", "world", "prod")])
        .unwrap();
    assert_eq!(index.len(), 1);
}

#[test]
fn failed_multi_record_encoding_keeps_every_previous_version() {
    let schema = schema();
    let mut index = SearchIndex::new(schema.clone(), 3, Some(Counting::new())).unwrap();
    let previous = vec![
        record(&schema, 1, "a", "old", "hello", "prod"),
        record(&schema, 2, "a", "old", "hello", "prod"),
    ];
    index.add(previous.clone()).unwrap();
    assert!(
        index
            .upsert(vec![
                record(&schema, 1, "b", "new", "world", "test"),
                record(&schema, 2, "b", "new", "fail", "test"),
                record(&schema, 3, "b", "new", "world", "test")
            ])
            .is_err()
    );
    assert_eq!(index.store().records(), previous);
    assert_eq!(index.len(), 2);
}

#[test]
fn semantic_report_budgets_slots_walked_after_removal() {
    let schema = schema();
    let mut index = SearchIndex::new(schema.clone(), 3, Some(Counting::new())).unwrap();
    index
        .add(
            (0..3)
                .map(|id| record(&schema, id, "a", "name", "hello", "prod"))
                .collect(),
        )
        .unwrap();
    index.remove(&[RecordId::Int(1), RecordId::Int(2)]).unwrap();
    // One active query walks three retained slots on each of two semantic fields.
    assert!(index.find(options(0.1), false, 0, 2).is_err());
    assert_eq!(index.find(options(0.1), false, 0, 6).unwrap().matches, []);
}
