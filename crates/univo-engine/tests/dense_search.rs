//! Embedding of records and exhaustive dense search, against a naive full
//! sort.

use std::cell::RefCell;
use std::sync::Mutex;

use univo_core::{Map, PreparedRecord, Schema, Value, prepare, semantic_similarity};
use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};
use univo_engine::{Neighbor, RecordVectors, embed_records, top_k, within};

const DIMENSION: usize = 4;

/// Maps each text to a unit vector with coordinates in {-2, ..., 2} before
/// normalization, so that equal similarities are frequent; "zzz" has no
/// known token. Records every batch it encodes.
struct GridEmbedder {
    descriptor: ModelDescriptor,
    batches: Mutex<Vec<Vec<String>>>,
}

impl GridEmbedder {
    fn new() -> Self {
        Self {
            descriptor: ModelDescriptor {
                source: ModelSource::Local {
                    path: "grid".into(),
                    files: Vec::new(),
                },
                dimension: DIMENSION,
                backend: "grid",
            },
            batches: Mutex::new(Vec::new()),
        }
    }

    fn vector(text: &str) -> [f32; DIMENSION] {
        if text == "zzz" {
            return [0.0; DIMENSION];
        }
        let mut state = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        });
        let mut values = [0.0f32; DIMENSION];
        for value in &mut values {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *value = f32::from(u8::try_from(state % 5).unwrap()) - 2.0;
        }
        if values.iter().all(|value| *value == 0.0) {
            values[0] = 1.0;
        }
        let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
        values.map(|value| value / norm)
    }
}

impl Embedder for GridEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        self.batches
            .lock()
            .unwrap()
            .push(texts.iter().map(|text| (*text).to_owned()).collect());
        let data = texts.iter().flat_map(|text| Self::vector(text)).collect();
        Embeddings::new("grid", data, DIMENSION)
    }
}

/// Records with one semantic text field.
fn semantic_schema() -> Schema {
    use univo_core::{FieldPath, Text, TextMode};
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

fn records(texts: &[&str]) -> Vec<PreparedRecord> {
    let schema = semantic_schema();
    texts
        .iter()
        .map(|text| {
            let value = Map::new(vec![("text".into(), Value::Str((*text).into()))]).unwrap();
            prepare(&schema, &Value::Map(value)).unwrap()
        })
        .collect()
}

fn texts(count: usize, seed: u64) -> Vec<String> {
    let state = RefCell::new(seed);
    (0..count)
        .map(|_| {
            let mut value = state.borrow_mut();
            *value ^= *value << 13;
            *value ^= *value >> 7;
            *value ^= *value << 17;
            format!("text {}", *value % 40)
        })
        .collect()
}

fn embedded(texts: &[String]) -> RecordVectors {
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    embed_records(&semantic_schema(), &records(&refs), &GridEmbedder::new()).unwrap()
}

#[test]
fn identical_texts_are_encoded_once_and_share_a_row() {
    let embedder = GridEmbedder::new();
    let records = records(&["Acme", "acme", "Globex", "zzz", "  ", "ACME"]);
    let vectors = embed_records(&semantic_schema(), &records, &embedder).unwrap();

    // Case folding makes the three spellings of acme one text.
    assert_eq!(
        *embedder.batches.lock().unwrap(),
        [vec![
            "acme".to_owned(),
            "globex".to_owned(),
            "zzz".to_owned()
        ]]
    );
    let field = vectors.field(0).unwrap();
    assert_eq!(field.len(), 6);
    assert_eq!(field.get(0), field.get(1));
    assert_eq!(
        field.get(0).unwrap().as_ptr(),
        field.get(5).unwrap().as_ptr()
    );
    assert_eq!(field.get(3), None, "a text without known tokens");
    assert_eq!(field.get(4), None, "a blank text");
    assert_eq!(field.get(0).unwrap(), GridEmbedder::vector("acme"));
    assert_eq!(
        vectors.record(2),
        [Some(&GridEmbedder::vector("globex")[..])]
    );
}

#[test]
fn only_semantic_text_fields_are_embedded() {
    use univo_core::{Exact, FieldPath, Text, TextMode};
    let path = |dotted| FieldPath::parse(dotted).unwrap();
    let text = |mode| Text {
        mode,
        ..Text::default()
    };
    let schema = Schema::builder()
        .text(path("lexical"), text(TextMode::Lexical))
        .text(path("semantic"), text(TextMode::Semantic))
        .exact(path("code"), Exact::default())
        .text(path("hybrid"), text(TextMode::Hybrid))
        .build()
        .unwrap();
    let value = Value::Map(
        Map::new(
            ["lexical", "semantic", "code", "hybrid"]
                .map(|key| (key.to_owned(), Value::Str(format!("{key} value"))))
                .to_vec(),
        )
        .unwrap(),
    );
    let records = vec![prepare(&schema, &value).unwrap()];
    let embedder = GridEmbedder::new();
    let vectors = embed_records(&schema, &records, &embedder).unwrap();
    let present: Vec<bool> = vectors.record(0).iter().map(Option::is_some).collect();
    assert_eq!(present, [false, true, false, false]);
    assert_eq!(embedder.batches.lock().unwrap().len(), 1);
}

#[test]
fn later_batches_extend_the_slots() {
    let first = texts(30, 1);
    let second = texts(20, 2);
    let mut vectors = embedded(&first);
    vectors.extend(embedded(&second)).unwrap();
    let all: Vec<String> = first.iter().chain(&second).cloned().collect();
    let expected = embedded(&all);
    for slot in 0..all.len() {
        assert_eq!(vectors.record(slot), expected.record(slot), "{slot}");
    }
}

/// Every candidate with its similarity, sorted as the contract says.
fn naive(
    vectors: &RecordVectors,
    query: &[f32],
    candidates: &[usize],
    key: impl Fn(usize) -> usize,
) -> Vec<Neighbor> {
    let field = vectors.field(0).unwrap();
    let mut all: Vec<Neighbor> = candidates
        .iter()
        .filter_map(|&slot| {
            Some(Neighbor {
                slot,
                similarity: semantic_similarity(query, field.get(slot)?),
            })
        })
        .collect();
    all.sort_by(|a, b| {
        b.similarity
            .total_cmp(&a.similarity)
            .then_with(|| key(a.slot).cmp(&key(b.slot)))
    });
    all
}

#[test]
fn top_k_and_within_agree_with_a_full_sort() {
    let mut corpus = texts(400, 7);
    corpus.extend(["zzz".to_owned(), "zzz".to_owned()]);
    let vectors = embedded(&corpus);
    let field = vectors.field(0).unwrap();
    let candidates: Vec<usize> = (0..corpus.len()).rev().step_by(2).collect();
    let keys: [&dyn Fn(usize) -> usize; 2] = [&|slot| slot, &|slot| usize::MAX - slot];

    for query_slot in [0, 3, 17, 250] {
        let query = field.get(query_slot).unwrap();
        for key in keys {
            let expected = naive(&vectors, query, &candidates, key);
            for k in [0, 1, 5, 37, 1000] {
                let found = top_k(field, query, candidates.iter().copied(), k, key);
                assert_eq!(found, expected[..k.min(expected.len())], "k={k}");
            }
            for threshold in [0.0, 0.5, 0.9, 1.0] {
                let found = within(field, query, candidates.iter().copied(), threshold, key);
                let filtered: Vec<Neighbor> = expected
                    .iter()
                    .copied()
                    .filter(|neighbor| neighbor.similarity >= threshold)
                    .collect();
                assert_eq!(found, filtered, "threshold={threshold}");
            }
        }
    }
}

/// Unit vectors of the default model's dimension: texts "b v" share a base
/// `b` and differ by a small perturbation `v`, so that similarities crowd
/// near 1, where the int8 bound is tightest.
struct NearEmbedder(ModelDescriptor);

impl NearEmbedder {
    const DIMENSION: usize = 128;

    fn noise(seed: u64, scale: f32) -> impl Iterator<Item = f32> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..Self::DIMENSION).map(move |_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (f32::from(u16::try_from(state % 2001).unwrap()) / 1000.0 - 1.0) * scale
        })
    }
}

impl Embedder for NearEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.0
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        let mut data = Vec::new();
        for text in texts {
            let (base, variant) = text.split_once(' ').unwrap();
            let (base, variant): (u64, u64) = (base.parse().unwrap(), variant.parse().unwrap());
            #[allow(clippy::cast_precision_loss)]
            let scale = variant as f32 * 0.01;
            let row: Vec<f32> = Self::noise(base, 1.0)
                .zip(Self::noise(1000 + variant, scale))
                .map(|(value, noise)| value + noise)
                .collect();
            let norm = row.iter().map(|value| value * value).sum::<f32>().sqrt();
            data.extend(row.iter().map(|value| value / norm));
        }
        Embeddings::new("near", data, Self::DIMENSION)
    }
}

#[test]
fn candidates_exactly_at_the_threshold_are_kept() {
    let corpus: Vec<String> = (0..6)
        .flat_map(|base| (0..40).map(move |variant| format!("{base} {variant}")))
        .collect();
    let refs: Vec<&str> = corpus.iter().map(String::as_str).collect();
    let embedder = NearEmbedder(ModelDescriptor {
        source: ModelSource::Local {
            path: "near".into(),
            files: Vec::new(),
        },
        dimension: NearEmbedder::DIMENSION,
        backend: "near",
    });
    let vectors = embed_records(&semantic_schema(), &records(&refs), &embedder).unwrap();
    let field = vectors.field(0).unwrap();
    let candidates: Vec<usize> = (0..corpus.len()).collect();

    for query_slot in [0, 39, 85, 200] {
        let query = field.get(query_slot).unwrap();
        let expected = naive(&vectors, query, &candidates, |slot| slot);
        assert!(expected.iter().filter(|n| n.similarity > 0.9).count() > 20);
        // Every similarity of the query, as a threshold, keeps its candidate.
        for threshold in expected.iter().map(|neighbor| neighbor.similarity) {
            let found = within(field, query, 0..corpus.len(), threshold, |slot| slot);
            let filtered: Vec<Neighbor> = expected
                .iter()
                .copied()
                .filter(|neighbor| neighbor.similarity >= threshold)
                .collect();
            assert_eq!(found, filtered, "threshold={threshold}");
        }
        for k in [1, 10, 40, 41, 100] {
            let found = top_k(field, query, 0..corpus.len(), k, |slot| slot);
            assert_eq!(found, expected[..k], "k={k}");
        }
    }
}

#[test]
fn results_do_not_depend_on_candidate_order() {
    let vectors = embedded(&texts(300, 11));
    let field = vectors.field(0).unwrap();
    let query = field.get(5).unwrap();
    let forward = top_k(field, query, 0..300, 25, |slot| slot);
    let backward = top_k(field, query, (0..300).rev(), 25, |slot| slot);
    assert_eq!(forward, backward);
    assert!(
        forward
            .windows(2)
            .any(|pair| pair[0].similarity.total_cmp(&pair[1].similarity).is_eq()),
        "the data must exercise ties"
    );
}

#[test]
fn queries_of_another_dimension_find_nothing() {
    let vectors = embedded(&texts(10, 3));
    let field = vectors.field(0).unwrap();
    assert_eq!(top_k(field, &[1.0, 0.0], 0..10, 5, |slot| slot), []);
    assert_eq!(within(field, &[1.0, 0.0], 0..10, 0.0, |slot| slot), []);
}

/// Another embedder: a different model, a wrong number of rows, or rows of
/// another dimension than it declares.
struct Faulty {
    descriptor: ModelDescriptor,
    rows: fn(usize) -> usize,
    row_dimension: usize,
}

impl Faulty {
    fn new(name: &str, dimension: usize, rows: fn(usize) -> usize, row_dimension: usize) -> Self {
        Self {
            descriptor: ModelDescriptor {
                source: ModelSource::Local {
                    path: name.into(),
                    files: Vec::new(),
                },
                dimension,
                backend: "faulty",
            },
            rows,
            row_dimension,
        }
    }
}

impl Embedder for Faulty {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        let mut row = vec![0.0; self.row_dimension];
        row[0] = 1.0;
        let data = (0..(self.rows)(texts.len()))
            .flat_map(|_| row.clone())
            .collect();
        Embeddings::new("faulty", data, self.row_dimension)
    }
}

#[test]
fn batches_of_another_model_or_schema_are_refused_unchanged() {
    use univo_core::ErrorCode;
    let records = records(&["acme", "globex"]);
    let schema = semantic_schema();
    let mut vectors = embed_records(&schema, &records, &GridEmbedder::new()).unwrap();
    let before = vectors.clone();

    for other in [
        Faulty::new("grid", 2, |rows| rows, 2),
        Faulty::new("other", DIMENSION, |rows| rows, DIMENSION),
    ] {
        let more = embed_records(&schema, &records, &other).unwrap();
        let error = vectors.extend(more).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ModelMismatch, "{error}");
        assert_eq!(vectors, before);
    }

    let strings = Schema::strings();
    let plain = vec![prepare(&strings, &Value::Str("acme".into())).unwrap()];
    let more = embed_records(&strings, &plain, &GridEmbedder::new()).unwrap();
    let error = vectors.extend(more).unwrap_err();
    assert_eq!(error.code(), ErrorCode::SchemaMismatch);
    assert_eq!(vectors, before);
}

#[test]
fn records_of_another_schema_and_faulty_embedders_are_reported() {
    use univo_core::{ErrorCode, FieldPath, Text, TextMode};
    let two_fields = Schema::builder()
        .text(
            FieldPath::parse("text").unwrap(),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .text(
            FieldPath::parse("other").unwrap(),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .build()
        .unwrap();
    let records = records(&["acme", "globex"]);
    let error = embed_records(&two_fields, &records, &GridEmbedder::new()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::SchemaMismatch);

    let schema = semantic_schema();
    for faulty in [
        Faulty::new("short", DIMENSION, |rows| rows - 1, DIMENSION),
        Faulty::new("narrow", DIMENSION, |rows| rows, 2),
    ] {
        let error = embed_records(&schema, &records, &faulty).unwrap_err();
        assert_eq!(error.code(), ErrorCode::CorruptModel, "{error}");
    }
}

#[test]
fn equal_similarities_and_keys_are_ordered_by_slot() {
    let vectors = embedded(&["same".to_owned(), "same".to_owned(), "same".to_owned()]);
    let field = vectors.field(0).unwrap();
    let query = field.get(0).unwrap();
    for candidates in [vec![0, 1, 2], vec![2, 1, 0], vec![1, 2, 0]] {
        let found = top_k(field, query, candidates.iter().copied(), 1, |_| ());
        assert_eq!(found[0].slot, 0);
        let all = within(field, query, candidates.iter().copied(), 0.0, |_| ());
        let slots: Vec<usize> = all.iter().map(|neighbor| neighbor.slot).collect();
        assert_eq!(slots, [0, 1, 2]);
    }
}
