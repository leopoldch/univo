//! `check_or_add`: one decision and at most one insertion per duplicate,
//! including when threads race on one index behind a lock.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, RwLock};

use univo_core::{
    ErrorCode, FieldPath, Map, PreparedRecord, Schema, Text, TextMode, Value, prepare,
};
use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};
use univo_engine::{Decision, SearchIndex, SearchOptions};

/// Two-dimensional unit vectors from the first letter of a text, counting
/// the texts it encodes.
struct LetterEmbedder {
    descriptor: ModelDescriptor,
    encoded: AtomicUsize,
}

impl LetterEmbedder {
    fn shared(path: &str) -> Arc<Self> {
        Arc::new(Self {
            descriptor: ModelDescriptor {
                source: ModelSource::Local {
                    path: path.into(),
                    files: Vec::new(),
                },
                dimension: 2,
                backend: "letter",
            },
            encoded: AtomicUsize::new(0),
        })
    }
}

impl Embedder for LetterEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        self.encoded.fetch_add(texts.len(), Ordering::Relaxed);
        let data = texts
            .iter()
            .flat_map(|text| {
                let angle = f32::from(text.bytes().next().unwrap_or(b'a')) / 10.0;
                [angle.cos(), angle.sin()]
            })
            .collect();
        Embeddings::new("letter", data, 2)
    }
}

fn path(dotted: &str) -> FieldPath {
    FieldPath::parse(dotted).unwrap()
}

fn schema() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .text(path("name"), Text::default())
        .text(
            path("description"),
            Text {
                mode: TextMode::Semantic,
                weight: 0.5,
                ..Text::default()
            },
        )
        .build()
        .unwrap()
}

fn record(schema: &Schema, id: i64, name: &str, description: &str) -> PreparedRecord {
    let entries = vec![
        ("id".to_owned(), Value::Int(id)),
        ("name".to_owned(), Value::Str(name.into())),
        ("description".to_owned(), Value::Str(description.into())),
    ];
    prepare(schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
}

fn options() -> SearchOptions {
    SearchOptions {
        min_score: 0.8,
        limit: 10,
        max_entries: usize::MAX,
    }
}

fn index(embedder: &Arc<LetterEmbedder>) -> SearchIndex {
    let model: Arc<dyn Embedder> = embedder.clone();
    SearchIndex::new(schema(), 1000, Some(model)).unwrap()
}

fn check(index: &mut SearchIndex, record: PreparedRecord) -> univo_engine::CheckOrAdd {
    let vectors = index.encoder().encode(&record).unwrap();
    index
        .check_or_add(record, vectors, options(), false)
        .unwrap()
}

#[test]
fn inserts_new_records_and_reports_equal_and_similar_ones() {
    let schema = schema();
    let embedder = LetterEmbedder::shared("letter");
    let mut index = index(&embedder);

    let first = check(
        &mut index,
        record(&schema, 1, "Acme Corporation", "cloud hosting"),
    );
    assert_eq!((first.decision, first.slot), (Decision::Inserted, Some(0)));
    assert_eq!(first.result.total_matches, 0);

    let equal = check(
        &mut index,
        record(&schema, 2, "ACME corporation", "Cloud hosting"),
    );
    assert_eq!((equal.decision, equal.slot), (Decision::Matched, None));
    assert_eq!(equal.result.matches[0].slot, 0);

    let similar = check(
        &mut index,
        record(&schema, 3, "Acme Corporatoin", "cloud hosting"),
    );
    assert_eq!((similar.decision, similar.slot), (Decision::Review, None));

    let other = check(&mut index, record(&schema, 4, "Globex", "retail"));
    assert_eq!((other.decision, other.slot), (Decision::Inserted, Some(1)));
    assert_eq!(index.len(), 2);
}

#[test]
fn the_query_is_encoded_once_for_the_search_and_the_insertion() {
    let schema = schema();
    let embedder = LetterEmbedder::shared("letter");
    let mut index = index(&embedder);
    let before = embedder.encoded.load(Ordering::Relaxed);
    check(&mut index, record(&schema, 1, "Acme", "cloud hosting"));
    assert_eq!(embedder.encoded.load(Ordering::Relaxed) - before, 1);
}

#[test]
fn foreign_embeddings_and_existing_ids_leave_the_index_unchanged() {
    let schema = schema();
    let embedder = LetterEmbedder::shared("letter");
    let mut index = index(&embedder);
    check(&mut index, record(&schema, 1, "Acme", "cloud hosting"));

    let other: Arc<dyn Embedder> = LetterEmbedder::shared("other");
    let foreign_index = SearchIndex::new(schema.clone(), 10, Some(other)).unwrap();
    let query = record(&schema, 2, "Globex", "retail");
    let foreign = foreign_index.encoder().encode(&query).unwrap();
    let error = index
        .check_or_add(query.clone(), foreign, options(), false)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelMismatch);
    let error = index
        .check_or_add(query, None, options(), false)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelMismatch);

    let existing = record(&schema, 1, "Initech", "consulting");
    let vectors = index.encoder().encode(&existing).unwrap();
    let error = index
        .check_or_add(existing, vectors, options(), false)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::DuplicateId);
    assert_eq!(index.len(), 1);
}

#[test]
fn work_limits_are_reported_with_the_decision() {
    let schema = schema();
    let embedder = LetterEmbedder::shared("letter");
    let mut index = index(&embedder);
    // Similar records, added directly: check_or_add would keep only one.
    index
        .add(
            (0..50)
                .map(|id| record(&schema, id, &format!("Name {id}"), "cloud hosting"))
                .collect(),
        )
        .unwrap();
    let query = record(&schema, 100, "Name 7", "cloud hosting");
    let vectors = index.encoder().encode(&query).unwrap();
    let bounded = SearchOptions {
        max_entries: 1,
        ..options()
    };
    let result = index.check_or_add(query, vectors, bounded, false).unwrap();
    assert!(result.result.truncated, "{result:?}");
}

const NAMES: &[&str] = &[
    "Acme Corporation",
    "Globex Industries",
    "Initech Software",
    "Umbrella Biotech",
    "Stark Engineering",
    "Wayne Logistics",
    "Tyrell Robotics",
    "Cyberdyne Systems",
    "Soylent Foods",
    "Wonka Confectionery",
];

#[test]
fn racing_threads_insert_a_duplicate_once() {
    let schema = schema();
    let embedder = LetterEmbedder::shared("letter");
    let index = Arc::new(RwLock::new(index(&embedder)));
    let threads = 8;
    for (round, base) in NAMES.iter().enumerate() {
        let barrier = Arc::new(Barrier::new(threads));
        let handles: Vec<_> = (0..threads)
            .map(|thread| {
                let (index, barrier, schema) = (index.clone(), barrier.clone(), schema.clone());
                std::thread::spawn(move || {
                    // Same name in every thread, a typo in odd ones; names
                    // of different rounds share nothing.
                    let name = if thread % 2 == 0 {
                        (*base).to_owned()
                    } else {
                        format!("{base}e")
                    };
                    let id = i64::try_from(round * 100 + thread).unwrap();
                    let query = record(&schema, id, &name, "cloud hosting");
                    let encoder = index.read().unwrap().encoder();
                    barrier.wait();
                    let vectors = encoder.encode(&query).unwrap();
                    let mut index = index.write().unwrap();
                    index
                        .check_or_add(query, vectors, options(), false)
                        .unwrap()
                        .decision
                })
            })
            .collect();
        let decisions: Vec<Decision> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let inserted = decisions
            .iter()
            .filter(|d| **d == Decision::Inserted)
            .count();
        assert_eq!(inserted, 1, "round {round}: {decisions:?}");
    }
    assert_eq!(index.read().unwrap().len(), NAMES.len());
}
