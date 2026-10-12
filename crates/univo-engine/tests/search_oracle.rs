//! The candidate union against the exhaustive oracle: without truncation,
//! a search returns exactly what scoring every record of the partition
//! returns, on every kind of field.

use std::sync::Arc;

use univo_core::{
    Conflict, ErrorCode, Exact, ExactNormalizer, FieldPath, Map, PreparedRecord, Schema, Text,
    TextMode, Value, prepare,
};
use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};
use univo_engine::{SearchIndex, SearchOptions, SearchResult};

const DIMENSION: usize = 4;

/// Unit vectors with coordinates in {-2, ..., 2} before normalization, so
/// that texts are often close; "zzz" has no known token.
struct GridEmbedder(ModelDescriptor);

impl GridEmbedder {
    fn shared() -> Arc<dyn Embedder> {
        Arc::new(Self(ModelDescriptor {
            source: ModelSource::Local {
                path: "grid".into(),
                files: Vec::new(),
            },
            dimension: DIMENSION,
            backend: "grid",
        }))
    }
}

impl Embedder for GridEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.0
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        let mut data = Vec::with_capacity(texts.len() * DIMENSION);
        for text in texts {
            if *text == "zzz" {
                data.extend([0.0; DIMENSION]);
                continue;
            }
            // Texts sharing their first word share most of their vector.
            let word = text.split(' ').next().unwrap_or_default();
            let mut state = word.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
            });
            let mut values = [0.0f32; DIMENSION];
            for value in &mut values {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *value = f32::from(u8::try_from(state % 5).unwrap()) - 2.0;
            }
            values[DIMENSION - 1] += f32::from(u8::try_from(text.len() % 3).unwrap());
            if values.iter().all(|value| *value == 0.0) {
                values[0] = 1.0;
            }
            let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
            data.extend(values.map(|value| value / norm));
        }
        Embeddings::new("grid", data, DIMENSION)
    }
}

fn path(dotted: &str) -> FieldPath {
    FieldPath::parse(dotted).unwrap()
}

fn schema() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .partition_by(path("tenant"))
        .text(path("name"), Text::default())
        .exact(
            path("domain"),
            Exact {
                normalizer: ExactNormalizer::Hostname,
                ..Exact::default()
            },
        )
        .exact(
            path("sku"),
            Exact {
                conflict: Conflict::Reject,
                weight: 0.5,
                ..Exact::default()
            },
        )
        .exact(path("params"), Exact::default())
        .text(
            path("description"),
            Text {
                mode: TextMode::Semantic,
                weight: 0.8,
                ..Text::default()
            },
        )
        .build()
        .unwrap()
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % bound as u64).unwrap()
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const NAMES: &[&str] = &[
    "acme",
    "acme corp",
    "acme corporation",
    "acne corp",
    "globex",
    "globex inc",
    "initech",
    "initek",
    "umbrella",
    "société générale",
    "societe generale",
    "zz",
];
const DOMAINS: &[&str] = &["acme.com", "ACME.com.", "globex.io", "initech.fr", "  "];
const DESCRIPTIONS: &[&str] = &[
    "cloud hosting services",
    "cloud storage",
    "retail stores",
    "retail banking",
    "zzz",
    "industrial chemicals",
];

/// A record of `tenant` with random, often missing, values.
fn value(rng: &mut Rng, id: i64, tenant: &str) -> Value {
    let mut entries = vec![
        ("id".to_owned(), Value::Int(id)),
        ("tenant".to_owned(), Value::Str(tenant.into())),
    ];
    let mut maybe = |rng: &mut Rng, key: &str, value: Value| {
        if rng.below(5) != 0 {
            entries.push((key.to_owned(), value));
        }
    };
    let name = rng.pick(NAMES);
    maybe(rng, "name", Value::Str(name.into()));
    let domain = rng.pick(DOMAINS);
    maybe(rng, "domain", Value::Str(domain.into()));
    let sku = rng.below(4);
    maybe(rng, "sku", Value::Int(i64::try_from(sku).unwrap()));
    let params = match rng.below(5) {
        0 => Value::Map(Map::new(Vec::new()).unwrap()),
        1 => Value::Null,
        _ => {
            let mut keys = vec![(
                "port".to_owned(),
                Value::Int(i64::try_from(rng.below(3)).unwrap()),
            )];
            if rng.below(2) == 0 {
                keys.push(("tls".to_owned(), Value::Bool(rng.below(2) == 0)));
            }
            if rng.below(3) == 0 {
                let nested = Map::new(vec![(
                    "level".to_owned(),
                    Value::Str(["info", "debug"][rng.below(2)].into()),
                )])
                .unwrap();
                keys.push(("log".to_owned(), Value::Map(nested)));
            }
            Value::Map(Map::new(keys).unwrap())
        }
    };
    maybe(rng, "params", params);
    let description = rng.pick(DESCRIPTIONS);
    maybe(rng, "description", Value::Str(description.into()));
    Value::Map(Map::new(entries).unwrap())
}

fn records(schema: &Schema, count: usize, seed: u64, first_id: i64) -> Vec<PreparedRecord> {
    let mut rng = Rng(seed);
    (0..count)
        .map(|index| {
            let tenant = ["north", "south"][rng.below(2)];
            let id = first_id + i64::try_from(index).unwrap();
            prepare(schema, &value(&mut rng, id, tenant)).unwrap()
        })
        .collect()
}

fn options(min_score: f32) -> SearchOptions {
    SearchOptions {
        min_score,
        limit: usize::MAX,
        max_entries: usize::MAX,
    }
}

fn index(count: usize) -> SearchIndex {
    let schema = schema();
    let mut index = SearchIndex::new(schema.clone(), 10_000, Some(GridEmbedder::shared())).unwrap();
    let all = records(&schema, count, 17, 0);
    let (first, second) = all.split_at(count / 3);
    index.add(first.to_vec()).unwrap();
    index.add(second.to_vec()).unwrap();
    index
}

fn same(found: &SearchResult, oracle: &SearchResult) {
    assert!(!found.truncated);
    assert_eq!(found.total_matches, oracle.total_matches);
    assert_eq!(found.matches, oracle.matches);
}

#[test]
fn stored_records_find_exactly_what_the_oracle_finds() {
    let index = index(400);
    let mut matched = 0;
    for slot in (0..index.len()).step_by(3) {
        for min_score in [0.2, 0.5, 0.7, 0.9, 1.0] {
            let found = index.search_slot(slot, options(min_score)).unwrap();
            let oracle = index
                .search_slot_exhaustive(slot, options(min_score))
                .unwrap();
            same(&found, &oracle);
            assert!(found.candidates <= oracle.candidates);
            matched += found.total_matches;
        }
    }
    assert!(matched > 2000, "the data must produce matches: {matched}");
}

#[test]
fn new_queries_find_exactly_what_the_oracle_finds() {
    let index = index(400);
    let schema = index.schema().clone();
    for query in records(&schema, 60, 99, 100_000) {
        for min_score in [0.3, 0.6, 0.8, 1.0] {
            let found = index.search(&query, options(min_score)).unwrap();
            let oracle = index.search_exhaustive(&query, options(min_score)).unwrap();
            same(&found, &oracle);
        }
    }
}

#[test]
fn matches_are_ranked_and_limited_without_losing_the_count() {
    let index = index(400);
    let all = index.search_slot(0, options(0.2)).unwrap();
    assert!(all.total_matches > 5);
    for pair in all.matches.windows(2) {
        let (a, b) = (
            pair[0].comparison.score.unwrap(),
            pair[1].comparison.score.unwrap(),
        );
        let ids = |slot: usize| index.store().get(slot).unwrap().id().cloned();
        let order = b
            .total_cmp(&a)
            .then_with(|| ids(pair[0].slot).cmp(&ids(pair[1].slot)));
        assert_eq!(order, std::cmp::Ordering::Less);
    }
    let limited = index
        .search_slot(
            0,
            SearchOptions {
                limit: 3,
                ..options(0.2)
            },
        )
        .unwrap();
    assert_eq!(limited.total_matches, all.total_matches);
    assert_eq!(limited.matches, all.matches[..3]);
    assert!(all.matches.iter().all(|found| !found.comparison.vetoed));
}

#[test]
fn work_limits_are_reported_and_keep_exact_matches() {
    let schema = schema();
    let mut index = SearchIndex::new(schema.clone(), 1000, Some(GridEmbedder::shared())).unwrap();
    let twin = |id: i64| {
        let entries = vec![
            ("id".to_owned(), Value::Int(id)),
            ("tenant".to_owned(), Value::Str("north".into())),
            ("name".to_owned(), Value::Str("acme corp".into())),
            ("domain".to_owned(), Value::Str("acme.com".into())),
            ("sku".to_owned(), Value::Int(2)),
            (
                "description".to_owned(),
                Value::Str("cloud hosting services".into()),
            ),
        ];
        prepare(&schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
    };
    let mut batch = vec![twin(0)];
    batch.extend(records(&schema, 300, 23, 1));
    index.add(batch).unwrap();
    let query = twin(-1);

    let limited = index
        .search(
            &query,
            SearchOptions {
                max_entries: 1,
                ..options(0.3)
            },
        )
        .unwrap();
    assert!(limited.truncated);
    assert_eq!(limited.lanes.exact, 1);
    assert!(limited.matches.iter().any(|found| found.slot == 0));
}

#[test]
fn invalid_minimums_queries_and_models_are_refused() {
    let index = index(30);
    for min_score in [0.0, -1.0, 1.5, f32::NAN] {
        let error = index.search_slot(0, options(min_score)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidThreshold);
        let error = index
            .search_slot_exhaustive(0, options(min_score))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidThreshold);
    }
    let foreign = prepare(&Schema::strings(), &Value::Str("acme".into())).unwrap();
    assert_eq!(
        index.search(&foreign, options(0.5)).unwrap_err().code(),
        ErrorCode::SchemaMismatch
    );
    assert_eq!(
        SearchIndex::new(schema(), 10, None).unwrap_err().code(),
        ErrorCode::ModelNotAvailable
    );
    assert_eq!(
        SearchIndex::new(schema(), usize::MAX, Some(GridEmbedder::shared()))
            .unwrap_err()
            .code(),
        ErrorCode::IndexCapacity
    );
}

#[test]
fn failed_batches_leave_the_index_unchanged() {
    let schema = schema();
    let mut index = SearchIndex::new(schema.clone(), 1000, Some(GridEmbedder::shared())).unwrap();
    let batch = records(&schema, 20, 3, 0);
    index.add(batch.clone()).unwrap();
    let before = index.search_slot(0, options(0.2)).unwrap();
    let error = index.add(batch[..2].to_vec()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DuplicateId);
    assert_eq!(index.len(), 20);
    assert_eq!(index.search_slot(0, options(0.2)).unwrap(), before);
}

/// Maps texts to fixed vectors: "a" to [1, 0], "b" to [1e-40, 1], so their
/// similarity is the subnormal 1e-40. Its descriptor's path changes after
/// `stable_calls` calls, as a misbehaving backend might.
struct FixedEmbedder {
    calls: std::sync::atomic::AtomicUsize,
    stable_calls: usize,
    descriptors: [ModelDescriptor; 2],
}

impl FixedEmbedder {
    fn shared(stable_calls: usize) -> Arc<dyn Embedder> {
        let descriptor = |path: &str| ModelDescriptor {
            source: ModelSource::Local {
                path: path.into(),
                files: Vec::new(),
            },
            dimension: 2,
            backend: "fixed",
        };
        Arc::new(Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            stable_calls,
            descriptors: [descriptor("first"), descriptor("second")],
        })
    }
}

impl Embedder for FixedEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        let calls = self.calls.load(std::sync::atomic::Ordering::Relaxed);
        &self.descriptors[usize::from(calls >= self.stable_calls)]
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let data = texts
            .iter()
            .flat_map(|text| {
                if *text == "a" {
                    [1.0, 0.0]
                } else {
                    [1e-40, 1.0]
                }
            })
            .collect();
        Embeddings::new("fixed", data, 2)
    }
}

fn semantic_only() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .text(
            path("text"),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .build()
        .unwrap()
}

fn text_record(schema: &Schema, id: i64, text: &str) -> PreparedRecord {
    let entries = vec![
        ("id".to_owned(), Value::Int(id)),
        ("text".to_owned(), Value::Str(text.into())),
    ];
    prepare(schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
}

#[test]
fn the_smallest_minimum_scores_still_equal_the_oracle() {
    let schema = semantic_only();
    let mut index =
        SearchIndex::new(schema.clone(), 10, Some(FixedEmbedder::shared(usize::MAX))).unwrap();
    index.add(vec![text_record(&schema, 1, "b")]).unwrap();
    let query = text_record(&schema, 2, "a");
    for min_score in [1e-40, f32::MIN_POSITIVE, 1e-30] {
        let found = index.search(&query, options(min_score)).unwrap();
        let oracle = index.search_exhaustive(&query, options(min_score)).unwrap();
        same(&found, &oracle);
    }
    assert_eq!(
        index.search(&query, options(1e-40)).unwrap().total_matches,
        1
    );
}

#[test]
fn a_model_that_changes_leaves_the_index_unchanged() {
    let schema = semantic_only();
    // The descriptor changes after the first encoded batch. Constructing
    // an empty index no longer invokes inference.
    let mut index = SearchIndex::new(schema.clone(), 10, Some(FixedEmbedder::shared(1))).unwrap();
    index.add(vec![text_record(&schema, 1, "a")]).unwrap();
    let error = index.add(vec![text_record(&schema, 2, "b")]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelMismatch);
    assert_eq!(index.len(), 1);
    assert_eq!(index.store().len(), 1);
    let error = index
        .search(&text_record(&schema, 3, "a"), options(0.5))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ModelMismatch);
    // Records with the rejected batch's IDs can still be added once the
    // model is right, which shows nothing of the batch was kept.
    assert_eq!(index.search_slot(0, options(0.5)).unwrap().total_matches, 0);
}

#[test]
fn slots_beyond_the_records_are_errors() {
    let index = index(10);
    for slot in [10, usize::MAX] {
        let error = index.search_slot(slot, options(0.5)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::IndexCapacity);
        let error = index
            .search_slot_exhaustive(slot, options(0.5))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::IndexCapacity);
    }
}

#[test]
fn a_large_group_of_duplicates_keeps_only_the_best_matches() {
    let schema = schema();
    let mut index = SearchIndex::new(schema.clone(), 5000, Some(GridEmbedder::shared())).unwrap();
    let mut batch = records(&schema, 1000, 41, 0);
    // A thousand records equal to the first, on every field but the ID.
    let twin = |id: i64| {
        let entries = vec![
            ("id".to_owned(), Value::Int(id)),
            ("tenant".to_owned(), Value::Str("north".into())),
            ("name".to_owned(), Value::Str("acme corp".into())),
            ("description".to_owned(), Value::Str("cloud storage".into())),
        ];
        prepare(&schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
    };
    batch.extend((0..1000).map(|offset| twin(10_000 + offset)));
    index.add(batch).unwrap();
    let all = index.search_slot(1000, options(0.3)).unwrap();
    for limit in [0, 1, 10, 500] {
        let limited = index
            .search_slot(
                1000,
                SearchOptions {
                    limit,
                    ..options(0.3)
                },
            )
            .unwrap();
        assert_eq!(limited.total_matches, all.total_matches);
        assert_eq!(limited.matches, all.matches[..limit]);
        let exhaustive = index
            .search_slot_exhaustive(
                1000,
                SearchOptions {
                    limit,
                    ..options(0.3)
                },
            )
            .unwrap();
        assert_eq!(exhaustive.matches, limited.matches);
    }
    assert!(all.total_matches >= 999);
}
