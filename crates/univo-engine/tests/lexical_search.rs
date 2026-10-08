//! Lexical candidates against an exhaustive scan of every record: without a
//! work limit, the index must find exactly the records whose similarity
//! reaches the threshold.

use univo_core::{
    ErrorCode, FieldPath, FieldState, Map, PreparedRecord, Schema, Text, TextMode, Value,
    lexical_similarity, prepare,
};
use univo_engine::{LexicalIndex, LexicalMatch};

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
        let mut index = LexicalIndex::new(&schema, 0).unwrap();
        index.add(&records[..250]).unwrap();
        index.add(&records[250..]).unwrap();
        let queries = corpus(&schema, 80, 7);
        let mut matched = 0;
        for query in records.iter().step_by(7).chain(&queries) {
            for threshold in [0.05, 0.3, 0.5, 0.7, 0.8, 0.9, 1.0] {
                let result = index
                    .search(&records, query, threshold, usize::MAX)
                    .unwrap();
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
    let mut index = LexicalIndex::new(&schema, 0).unwrap();
    index.add(&records).unwrap();
    let query = record(&schema, "t1", "zürich insurance groupe");
    let result = index.search(&records, &query, 0.8, usize::MAX).unwrap();
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
    let mut index = LexicalIndex::new(&schema, 0).unwrap();
    index.add(&records).unwrap();
    let query = &records[0];
    let complete = index.search(&records, query, 0.3, usize::MAX).unwrap();
    let limited = index.search(&records, query, 0.3, 10).unwrap();
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
    let mut index = LexicalIndex::new(&schema, 0).unwrap();
    index.add(&records).unwrap();

    let found = index
        .search(&records, &records[0], 1.0, usize::MAX)
        .unwrap();
    let slots: Vec<usize> = found.matches.iter().map(|found| found.slot).collect();
    assert_eq!(slots, [0]);
    let blank = index
        .search(&records, &records[2], 0.1, usize::MAX)
        .unwrap();
    assert_eq!(blank.matches, []);

    let other = Schema::strings();
    let foreign = prepare(&other, &Value::Str("acme".into())).unwrap();
    assert_eq!(
        index
            .search(&records, &foreign, 0.5, usize::MAX)
            .unwrap_err()
            .code(),
        ErrorCode::SchemaMismatch
    );
    assert_eq!(
        index.add(&[foreign]).unwrap_err().code(),
        ErrorCode::SchemaMismatch
    );
    assert_eq!(index.len(), 3);
    assert_eq!(
        index
            .search(&records[..2], &records[0], 0.5, usize::MAX)
            .unwrap_err()
            .code(),
        ErrorCode::SchemaMismatch
    );
    let semantic = self::schema(TextMode::Semantic);
    assert_eq!(
        LexicalIndex::new(&semantic, 0).unwrap_err().code(),
        ErrorCode::SchemaMismatch
    );
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
    let mut index = LexicalIndex::new(&schema, 0).unwrap();
    index.add(&records).unwrap();
    let query = record(&schema, "t1", "abc");
    for threshold in [0.3, 0.5, 0.7] {
        let result = index
            .search(&records, &query, threshold, usize::MAX)
            .unwrap();
        assert_eq!(
            result.matches,
            scan(&records, &query, threshold),
            "{threshold}"
        );
    }
    let found = index.search(&records, &query, 0.3, usize::MAX).unwrap();
    assert!(found.matches.iter().any(|found| found.slot == 0));
}
