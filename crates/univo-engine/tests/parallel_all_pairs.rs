//! All-pairs reports run their queries in parallel: the result must not
//! depend on the number of threads, and must still equal the oracle.

use std::sync::Arc;

use univo_core::{
    Conflict, ErrorCode, Exact, FieldPath, Map, PreparedRecord, Schema, Text, TextMode, Value,
    prepare,
};
use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};
use univo_engine::{GroupOptions, SearchIndex, SearchOptions};

const DIMENSION: usize = 16;

/// Unit vectors summing hashed character bigrams: similar texts get close
/// vectors, and equal texts equal ones.
struct BigramEmbedder(ModelDescriptor);

impl Embedder for BigramEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.0
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        let mut data = Vec::with_capacity(texts.len() * DIMENSION);
        for text in texts {
            let mut vector = [0.0f32; DIMENSION];
            let bytes = text.as_bytes();
            for pair in bytes.windows(2) {
                let hash = usize::from(pair[0]) * 31 + usize::from(pair[1]);
                vector[hash % DIMENSION] += 1.0;
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

fn schema() -> Schema {
    Schema::builder()
        .id_field(path("id"))
        .partition_by(path("tenant"))
        .text(path("name"), Text::default())
        .text(
            path("description"),
            Text {
                mode: TextMode::Semantic,
                weight: 0.5,
                ..Text::default()
            },
        )
        .exact(
            path("kind"),
            Exact {
                conflict: Conflict::Reject,
                ..Exact::default()
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
}

const NAMES: &[&str] = &["acme corp", "globex", "initech", "umbrella", "stark"];
const DESCRIPTIONS: &[&str] = &["cloud hosting", "retail shop", "biotech lab", "robotics"];

/// Near-duplicates, exact copies (ties) and two partitions.
fn index(seed: u64) -> SearchIndex {
    let schema = schema();
    let mut rng = Rng(seed);
    let records: Vec<PreparedRecord> = (0..400)
        .map(|id| {
            let mut name: Vec<char> = NAMES[rng.below(NAMES.len())].chars().collect();
            for _ in 0..rng.below(3) {
                let position = rng.below(name.len());
                name[position] = ['a', 'e', 'x'][rng.below(3)];
            }
            let entries = vec![
                ("id".to_owned(), Value::Int(id)),
                (
                    "tenant".to_owned(),
                    Value::Str(["t1", "t2"][rng.below(2)].into()),
                ),
                ("name".to_owned(), Value::Str(name.into_iter().collect())),
                (
                    "description".to_owned(),
                    Value::Str(DESCRIPTIONS[rng.below(DESCRIPTIONS.len())].into()),
                ),
                (
                    "kind".to_owned(),
                    Value::Str(["a", "a", "b"][rng.below(3)].into()),
                ),
            ];
            prepare(&schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
        })
        .collect();
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
    index
}

fn on<T: Send>(threads: usize, work: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(work)
}

fn group_options(min_score: f32, exhaustive: bool, max_pairs: usize) -> GroupOptions {
    GroupOptions {
        min_score,
        max_entries: usize::MAX,
        exhaustive,
        max_group_size: 100,
        max_pairs,
        max_exhaustive_pairs: usize::MAX,
        max_semantic_comparisons: usize::MAX,
    }
}

#[test]
fn find_is_the_same_on_every_number_of_threads_and_equals_the_oracle() {
    for seed in [5, 41] {
        let index = index(seed);
        for min_score in [0.6, 0.85] {
            for limit in [0, 1, 7, 100, usize::MAX] {
                let options = SearchOptions {
                    min_score,
                    limit,
                    max_entries: usize::MAX,
                };
                let oracle = on(1, || index.find(options, true, usize::MAX, usize::MAX)).unwrap();
                assert!(oracle.total_matches > 100, "{}", oracle.total_matches);
                // Outside any pool, queries run in order on this thread.
                let plain = index.find(options, false, usize::MAX, usize::MAX).unwrap();
                assert_eq!(plain.matches, oracle.matches);
                for threads in [1, 2, 4, 8] {
                    for exhaustive in [false, true] {
                        let report = on(threads, || {
                            index.find(options, exhaustive, usize::MAX, usize::MAX)
                        })
                        .unwrap();
                        assert_eq!(report.matches, oracle.matches, "{threads} {exhaustive}");
                        assert_eq!(report.total_matches, oracle.total_matches);
                        assert!(!report.candidate_truncated);
                        if exhaustive {
                            assert_eq!(report.candidates, oracle.candidates);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn groups_are_the_same_on_every_number_of_threads() {
    let index = index(13);
    for min_score in [0.6, 0.85] {
        let expected = on(1, || {
            index.groups(group_options(min_score, true, usize::MAX))
        })
        .unwrap();
        assert!(expected.qualifying_pairs > 0);
        for threads in [1, 2, 4, 8] {
            for exhaustive in [false, true] {
                let report = on(threads, || {
                    index.groups(group_options(min_score, exhaustive, usize::MAX))
                })
                .unwrap();
                assert_eq!(report.exact, expected.exact);
                assert_eq!(report.groups, expected.groups);
                assert_eq!(report.qualifying_pairs, expected.qualifying_pairs);
                assert_eq!(report.split_pairs, expected.split_pairs);
            }
        }
    }
}

#[test]
fn the_group_pair_budget_fails_exactly_above_the_qualifying_pairs() {
    let index = index(29);
    let pairs = on(1, || index.groups(group_options(0.6, false, usize::MAX)))
        .unwrap()
        .qualifying_pairs;
    // More pairs than one worker buffers, so several folds race.
    assert!(pairs > 2 * 1024, "{pairs}");
    for threads in [1, 2, 8] {
        let fits = on(threads, || index.groups(group_options(0.6, false, pairs))).unwrap();
        assert_eq!(fits.qualifying_pairs, pairs);
        // Repeated: a worker folding after another one failed must not
        // insert more pairs, or the reported count would vary.
        // A limit reached while other workers are still folding, too.
        for limit in [pairs - 1, pairs / 4] {
            for _ in 0..50 {
                let error =
                    on(threads, || index.groups(group_options(0.6, false, limit))).unwrap_err();
                assert_eq!(error.code(), ErrorCode::GroupPairLimit, "{threads}");
                assert_eq!(
                    error.details(),
                    univo_core::Error::group_pair_limit(limit + 1, limit).details(),
                    "{threads} {limit}"
                );
            }
        }
    }
}

/// One field, so one lane must find every pair alone; duplicates sit in
/// adjacent slots, where an all-pairs query starts its scan.
#[test]
fn each_lane_alone_finds_the_pairs_of_adjacent_slots() {
    for mode in [TextMode::Lexical, TextMode::Semantic] {
        let schema = Schema::builder()
            .text(
                path("text"),
                Text {
                    mode,
                    ..Text::default()
                },
            )
            .build()
            .unwrap();
        let mut rng = Rng(7);
        let records: Vec<PreparedRecord> = (0..200)
            .flat_map(|_| {
                let base = format!("{} {}", NAMES[rng.below(NAMES.len())], rng.below(1000));
                [base.clone(), format!("{base}x")]
            })
            .map(|text| {
                let entries = vec![("text".to_owned(), Value::Str(text))];
                prepare(&schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
            })
            .collect();
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
        let options = SearchOptions {
            min_score: 0.8,
            limit: usize::MAX,
            max_entries: usize::MAX,
        };
        let oracle = on(1, || index.find(options, true, usize::MAX, usize::MAX)).unwrap();
        let adjacent = oracle
            .matches
            .iter()
            .filter(|found| found.right == found.left + 1)
            .count();
        assert!(adjacent >= 150, "{mode:?} {adjacent}");
        for threads in [1, 4] {
            let report = on(threads, || {
                index.find(options, false, usize::MAX, usize::MAX)
            })
            .unwrap();
            assert_eq!(report.matches, oracle.matches, "{mode:?} {threads}");
        }
    }
}
