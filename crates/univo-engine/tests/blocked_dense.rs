//! All-pairs reports compare blocks of queries with each vector read: they
//! must find exactly the pairs of the exhaustive oracle, across block and
//! partition boundaries, missing embeddings and removed records.

use std::sync::Arc;

use univo_core::{
    FieldPath, Map, PreparedRecord, RecordId, Schema, Text, TextMode, Value, prepare,
};
use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};
use univo_engine::{GroupOptions, SearchIndex, SearchOptions};

const DIMENSION: usize = 16;

/// Unit vectors summing hashed character bigrams.
struct BigramEmbedder(ModelDescriptor);

impl Embedder for BigramEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.0
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        let mut data = Vec::with_capacity(texts.len() * DIMENSION);
        for text in texts {
            let mut vector = [0.0f32; DIMENSION];
            for pair in text.as_bytes().windows(2) {
                vector[(usize::from(pair[0]) * 31 + usize::from(pair[1])) % DIMENSION] += 1.0;
            }
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            data.extend(
                vector
                    .iter()
                    .map(|x| if norm > 0.0 { x / norm } else { 0.0 }),
            );
        }
        Embeddings::new("bigram", data, DIMENSION)
    }
}

fn path(dotted: &str) -> FieldPath {
    FieldPath::parse(dotted).unwrap()
}

fn semantic(weight: f32) -> Text {
    Text {
        mode: TextMode::Semantic,
        weight,
        ..Text::default()
    }
}

/// Two semantic fields and no other lane but exact equality: the blocked
/// dense lane alone must find every pair.
fn schema() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .partition_by(path("tenant"))
        .text(path("name"), semantic(1.0))
        .text(path("description"), semantic(0.5))
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
}

const NAMES: &[&str] = &["acme corp", "globex", "initech", "umbrella", "stark"];
const DESCRIPTIONS: &[&str] = &["cloud hosting", "retail shop", "", "robotics lab"];

fn record(schema: &Schema, id: i64, tenant: &str, name: &str, description: &str) -> PreparedRecord {
    let entries = vec![
        ("id".to_owned(), Value::Int(id)),
        ("tenant".to_owned(), Value::Str(tenant.into())),
        ("name".to_owned(), Value::Str(name.into())),
        ("description".to_owned(), Value::Str(description.into())),
    ];
    prepare(schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
}

/// Partitions of 3 to about 200 records, so blocks are full, partial and
/// smaller than one group of four; copies share rows; some descriptions are
/// missing and some records are removed.
fn index(seed: u64) -> SearchIndex {
    let schema = schema();
    let mut rng = Rng(seed);
    let mut records = Vec::new();
    for id in 0..450 {
        let tenant = ["big", "big", "big", "mid", "tiny"][rng.below(5)];
        let tenant = if tenant == "tiny" && id % 7 != 0 {
            "mid"
        } else {
            tenant
        };
        let mut name: Vec<char> = NAMES[rng.below(NAMES.len())].chars().collect();
        for _ in 0..rng.below(3) {
            let position = rng.below(name.len());
            name[position] = ['a', 'e', 'x'][rng.below(3)];
        }
        let name: String = name.into_iter().collect();
        let description = DESCRIPTIONS[rng.below(DESCRIPTIONS.len())];
        records.push(record(&schema, id, tenant, &name, description));
    }
    let model: Arc<dyn Embedder> = Arc::new(BigramEmbedder(ModelDescriptor {
        source: ModelSource::Local {
            path: "bigram".into(),
            files: Vec::new(),
        },
        dimension: DIMENSION,
        backend: "bigram",
    }));
    let mut index = SearchIndex::new(schema, 10_000, Some(model)).unwrap();
    index.add(records).unwrap();
    let removed: Vec<RecordId> = (0..450).step_by(11).map(RecordId::Int).collect();
    index.remove(&removed).unwrap();
    index
}

fn on<T: Send>(threads: usize, work: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(work)
}

#[test]
fn blocked_find_equals_the_oracle() {
    for seed in [3, 19, 77] {
        let index = index(seed);
        for min_score in [0.5, 0.7, 0.9, 1.0] {
            let options = SearchOptions {
                min_score,
                limit: usize::MAX,
                max_entries: usize::MAX,
            };
            let oracle = index.find(options, true, usize::MAX, usize::MAX).unwrap();
            assert!(oracle.total_matches > 50, "{seed} {min_score}");
            for threads in [None, Some(1), Some(4)] {
                let run = || index.find(options, false, usize::MAX, usize::MAX).unwrap();
                let report = match threads {
                    None => run(),
                    Some(threads) => on(threads, run),
                };
                assert_eq!(
                    report.matches, oracle.matches,
                    "{seed} {min_score} {threads:?}"
                );
                assert_eq!(report.total_matches, oracle.total_matches);
            }
        }
    }
}

#[test]
fn blocked_groups_equal_exhaustive_groups() {
    let index = index(5);
    for min_score in [0.6, 0.85] {
        let options = GroupOptions {
            min_score,
            max_entries: usize::MAX,
            exhaustive: false,
            max_group_size: 100,
            max_pairs: usize::MAX,
            max_exhaustive_pairs: usize::MAX,
            max_semantic_comparisons: usize::MAX,
        };
        let oracle = index
            .groups(GroupOptions {
                exhaustive: true,
                ..options
            })
            .unwrap();
        let report = on(4, || index.groups(options)).unwrap();
        assert!(report.qualifying_pairs > 0);
        assert_eq!(report.qualifying_pairs, oracle.qualifying_pairs);
        assert_eq!(report.groups, oracle.groups);
        assert_eq!(report.exact, oracle.exact);
    }
}
