//! Lexical candidates against an exhaustive scan of every record: without a
//! work limit, the index must find exactly the records whose similarity
//! reaches the threshold.

use univo_core::{
    ErrorCode, FieldPath, FieldState, Map, PreparedRecord, Schema, Text, TextMode, Value,
    lexical_similarity, prepare,
};
use univo_engine::{LexicalIndex, LexicalMatch, RecordStore};

fn schema(mode: TextMode) -> Schema {
    Schema::builder()
        .partition_by(FieldPath::parse("tenant").unwrap())
        .text(
            FieldPath::parse("name").unwrap(),
            Text {
                mode,
                ..Text::default()
            },
        )
        .build()
        .unwrap()
}

fn record(schema: &Schema, tenant: &str, name: &str) -> PreparedRecord {
    let value = Map::new(vec![
        ("tenant".into(), Value::Str(tenant.into())),
        ("name".into(), Value::Str(name.into())),
    ])
    .unwrap();
    prepare(schema, &Value::Map(value)).unwrap()
}

/// A store holding `records` and an index of its first field.
fn indexed(schema: &Schema, records: &[PreparedRecord]) -> (RecordStore, LexicalIndex) {
    let mut store = RecordStore::new(schema.clone());
    store.append(records.to_vec()).unwrap();
    let index = LexicalIndex::new(&store, 0).unwrap();
    (store, index)
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

const BASES: &[&str] = &[
    "amazon",
    "acme corporation",
    "société générale",
    "ab",
    "a",
    "aaaa",
    "zürich insurance group",
    "international business machines corporation of new york and elsewhere in the world",
    "globex",
    "initech",
];
const LETTERS: &[char] = &['a', 'b', 'e', 'o', 'z', 'é', ' ', 'm'];

/// A base name with up to three random edits, or a blank text.
fn name(rng: &mut Rng) -> String {
    if rng.below(25) == 0 {
        return "   ".into();
    }
    let mut chars: Vec<char> = BASES[rng.below(BASES.len())].chars().collect();
    for _ in 0..rng.below(4) {
        let position = rng.below(chars.len() + 1);
        let letter = LETTERS[rng.below(LETTERS.len())];
        match rng.below(3) {
            0 => chars.insert(position, letter),
            1 if position < chars.len() => chars[position] = letter,
            _ if position < chars.len() => {
                chars.remove(position);
            }
            _ => chars.push(letter),
        }
    }
    chars.into_iter().collect()
}

fn corpus(schema: &Schema, count: usize, seed: u64) -> Vec<PreparedRecord> {
    let mut rng = Rng(seed);
    (0..count)
        .map(|_| {
            let tenant = ["t1", "t2", "t3"][rng.below(3)];
            record(schema, tenant, &name(&mut rng))
        })
        .collect()
}

fn text(record: &PreparedRecord) -> Option<(&str, &[u64])> {
    let field = &record.fields()[0];
    match &field.state {
        FieldState::Text(text) if !text.trim().is_empty() => Some((text, field.trigrams()?)),
        _ => None,
    }
}

/// Every record of the query's partition at or above the threshold.
fn scan(records: &[PreparedRecord], query: &PreparedRecord, threshold: f32) -> Vec<LexicalMatch> {
    let Some((query_text, query_trigrams)) = text(query) else {
        return Vec::new();
    };
    let mut found: Vec<LexicalMatch> = records
        .iter()
        .enumerate()
        .filter(|(_, record)| record.partition() == query.partition())
        .filter_map(|(slot, record)| {
            let (other, other_trigrams) = text(record)?;
            let (similarity, signal) =
                lexical_similarity(query_text, query_trigrams, other, other_trigrams);
            (similarity >= threshold).then_some(LexicalMatch {
                slot,
                similarity,
                signal,
            })
        })
        .collect();
    found.sort_by(|a, b| {
        b.similarity
            .total_cmp(&a.similarity)
            .then_with(|| a.slot.cmp(&b.slot))
    });
    found
}

#[test]
fn search_finds_exactly_what_an_exhaustive_scan_finds() {
    for mode in [TextMode::Lexical, TextMode::Hybrid] {
        let schema = schema(mode);
        let records = corpus(&schema, 600, 42);
        let mut store = RecordStore::new(schema.clone());
        store.append(records[..250].to_vec()).unwrap();
        let mut index = LexicalIndex::new(&store, 0).unwrap();
        store.append(records[250..].to_vec()).unwrap();
        index.update(&store).unwrap();
        let queries = corpus(&schema, 80, 7);
        let mut matched = 0;
        for query in records.iter().step_by(7).chain(&queries) {
            for threshold in [0.05, 0.3, 0.5, 0.7, 0.8, 0.9, 1.0] {
                let result = index.search(&store, query, threshold, usize::MAX).unwrap();
                assert!(!result.truncated);
                let expected = scan(&records, query, threshold);
                assert_eq!(result.matches, expected, "threshold {threshold}");
                matched += expected.len();
            }
        }
        assert!(matched > 1000, "the corpus must produce matches");
    }
}

#[test]
fn high_thresholds_read_far_less_than_a_scan() {
    let schema = schema(TextMode::Lexical);
    let records = corpus(&schema, 3000, 5);
    let (store, index) = indexed(&schema, &records);
    let query = record(&schema, "t1", "zürich insurance groupe");
    let result = index.search(&store, &query, 0.8, usize::MAX).unwrap();
    assert_ne!(result.matches, []);
    assert!(
        result.candidates_verified < records.len() / 10,
        "{} candidates verified",
        result.candidates_verified
    );
}

#[test]
fn the_work_limit_truncates_visibly() {
    let schema = schema(TextMode::Lexical);
    let records = corpus(&schema, 600, 9);
    let (store, index) = indexed(&schema, &records);
    let query = &records[0];
    let complete = index.search(&store, query, 0.3, usize::MAX).unwrap();
    let limited = index.search(&store, query, 0.3, 10).unwrap();
    assert!(limited.truncated);
    assert_eq!(limited.entries_read, 10);
    assert!(
        limited
            .matches
            .iter()
            .all(|found| complete.matches.contains(found))
    );
    assert!(limited.matches.len() < complete.matches.len());
}

#[test]
fn partitions_blank_texts_and_schemas_are_respected() {
    let schema = schema(TextMode::Lexical);
    let records = vec![
        record(&schema, "t1", "acme"),
        record(&schema, "t2", "acme"),
        record(&schema, "t1", "  "),
    ];
    let (mut store, index) = indexed(&schema, &records);

    let found = index.search(&store, &records[0], 1.0, usize::MAX).unwrap();
    let slots: Vec<usize> = found.matches.iter().map(|found| found.slot).collect();
    assert_eq!(slots, [0]);
    let blank = index.search(&store, &records[2], 0.1, usize::MAX).unwrap();
    assert_eq!(blank.matches, []);

    let foreign = prepare(&Schema::strings(), &Value::Str("acme".into())).unwrap();
    let error = index.search(&store, &foreign, 0.5, usize::MAX).unwrap_err();
    assert_eq!(error.code(), ErrorCode::SchemaMismatch);
    let error = store.append(vec![foreign]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::SchemaMismatch);
    assert_eq!(store.len(), 3);

    let semantic = RecordStore::new(self::schema(TextMode::Semantic));
    let error = LexicalIndex::new(&semantic, 0).unwrap_err();
    assert_eq!(error.code(), ErrorCode::SchemaMismatch);
}

#[test]
fn an_index_is_used_only_with_its_store_once_updated() {
    let schema = schema(TextMode::Lexical);
    let records = vec![
        record(&schema, "t1", "acme"),
        record(&schema, "t1", "globex"),
    ];
    let (mut store, mut index) = indexed(&schema, &records);

    // Same records, same length, another store: refused, never misread.
    let (other, _) = indexed(&schema, &[records[1].clone(), records[0].clone()]);
    let error = index
        .search(&other, &records[0], 1.0, usize::MAX)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::IndexOutOfSync);
    assert_eq!(
        index.update(&other).unwrap_err().code(),
        ErrorCode::IndexOutOfSync
    );

    store.append(vec![record(&schema, "t1", "acme")]).unwrap();
    let error = index
        .search(&store, &records[0], 1.0, usize::MAX)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::IndexOutOfSync);
    assert_eq!(error.details()[0].1, univo_core::Detail::Count(2));

    index.update(&store).unwrap();
    let found = index.search(&store, &records[0], 1.0, usize::MAX).unwrap();
    let slots: Vec<usize> = found.matches.iter().map(|found| found.slot).collect();
    assert_eq!(slots, [0, 2]);
}

#[test]
fn thresholds_outside_the_unit_interval_are_refused() {
    let schema = schema(TextMode::Lexical);
    let records = vec![record(&schema, "t1", "acme")];
    let (store, index) = indexed(&schema, &records);
    for threshold in [1.1, 0.0, -0.5, f32::NAN, f32::INFINITY] {
        let error = index
            .search(&store, &records[0], threshold, usize::MAX)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidThreshold, "{threshold}");
    }
    assert!(index.search(&store, &records[0], 1.0, usize::MAX).is_ok());
    assert!(
        index
            .search(&store, &records[0], f32::MIN_POSITIVE, usize::MAX)
            .is_ok()
    );
}

#[test]
fn small_partitions_of_a_large_index_are_searched_correctly() {
    // A small partition takes the hash-set path for already-seen records.
    let schema = schema(TextMode::Lexical);
    let mut records = corpus(&schema, 2000, 3);
    records.extend(
        ["acme", "acme corp", "acme corporation", "globex"]
            .iter()
            .map(|name| record(&schema, "small", name)),
    );
    let (store, index) = indexed(&schema, &records);
    let query = record(&schema, "small", "acme corp");
    for threshold in [0.3, 0.6, 0.9] {
        let result = index.search(&store, &query, threshold, usize::MAX).unwrap();
        assert_eq!(
            result.matches,
            scan(&records, &query, threshold),
            "{threshold}"
        );
    }
}

#[test]
fn short_strings_sharing_no_trigram_are_found_by_length() {
    // "abc" and "xbz" share no padded trigram, yet their edit similarity is
    // 1/3: only the scan by length can find them.
    let schema = schema(TextMode::Lexical);
    let records: Vec<PreparedRecord> = ["xbz", "abd", "xyzw", "abcdefgh"]
        .iter()
        .map(|name| record(&schema, "t1", name))
        .collect();
    let (store, index) = indexed(&schema, &records);
    let query = record(&schema, "t1", "abc");
    for threshold in [0.3, 0.5, 0.7] {
        let result = index.search(&store, &query, threshold, usize::MAX).unwrap();
        assert_eq!(
            result.matches,
            scan(&records, &query, threshold),
            "{threshold}"
        );
    }
    let found = index.search(&store, &query, 0.3, usize::MAX).unwrap();
    assert!(found.matches.iter().any(|found| found.slot == 0));
}
