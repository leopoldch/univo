//! Resource budget for lexical candidates, from `benchmarks/budgets.toml`:
//! names of about 20 characters, one partition, one thread. Release builds
//! only; shared CI runners multiply time budgets by `UNIVO_PERF_SLACK`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use univo_core::{FieldPath, Map, PreparedRecord, Schema, Text, TextMode, Value, prepare};
use univo_engine::{LexicalIndex, RecordStore};

/// Syllables of synthetic company names: a vocabulary of a few thousand
/// words, as diverse as real names, unlike a fixed list of words.
const SYLLABLES: &[&str] = &[
    "ka", "lo", "mi", "ne", "ra", "to", "vi", "su", "dor", "pel", "ban", "tri", "gon", "zu", "fa",
    "re", "qui", "mo", "sel", "an", "ex", "or", "il", "ty", "cha", "vel", "bro", "kin", "dal",
    "mer",
];

fn budget(key: &str) -> toml::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/budgets.toml");
    let budgets: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
    budgets["engine"]["lexical_search"][key].clone()
}

fn report(line: &str) {
    eprintln!("{line}");
    if let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(summary)
            .unwrap();
        writeln!(file, "- `{}`: {line}", std::env::consts::ARCH).unwrap();
    }
}

/// Names of two or three words of two or three syllables and a number,
/// about 20 characters, with a typo in one name out of ten.
fn names(count: usize) -> Vec<String> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move |bound: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        usize::try_from(state % bound as u64).unwrap()
    };
    (0..count)
        .map(|_| {
            let words: Vec<String> = (0..2 + next(2))
                .map(|_| {
                    (0..2 + next(2))
                        .map(|_| SYLLABLES[next(SYLLABLES.len())])
                        .collect()
                })
                .collect();
            let mut name = format!("{} {}", words.join(" "), next(1000));
            if next(10) == 0 {
                name.replace_range(0..1, "x");
            }
            name
        })
        .collect()
}

#[test]
#[allow(clippy::cast_precision_loss)]
#[cfg_attr(
    debug_assertions,
    ignore = "performance budgets apply to release builds"
)]
fn lexical_search_stays_within_budget() {
    let count = usize::try_from(budget("records").as_integer().unwrap()).unwrap();
    let slack: f64 = std::env::var("UNIVO_PERF_SLACK").map_or(1.0, |value| value.parse().unwrap());
    assert!(slack.is_finite() && slack >= 1.0);
    let limit =
        |key: &str| Duration::from_secs_f64(budget(key).as_float().unwrap() / 1000.0 * slack);

    let schema = Schema::builder()
        .text(
            FieldPath::parse("name").unwrap(),
            Text {
                mode: TextMode::Lexical,
                ..Text::default()
            },
        )
        .build()
        .unwrap();
    let records: Vec<PreparedRecord> = names(count)
        .iter()
        .map(|name| {
            let value = Map::new(vec![("name".into(), Value::Str(name.clone()))]).unwrap();
            prepare(&schema, &Value::Map(value)).unwrap()
        })
        .collect();

    let mut builds: Vec<Duration> = (0..3)
        .map(|_| {
            let mut store = RecordStore::new(schema.clone(), usize::MAX);
            store.append(records.clone()).unwrap();
            let start = Instant::now();
            let index = LexicalIndex::new(&store, 0).unwrap();
            let elapsed = start.elapsed();
            assert_eq!(std::hint::black_box(&index).len(), count);
            elapsed
        })
        .collect();
    builds.sort_unstable();
    let mut store = RecordStore::new(schema.clone(), usize::MAX);
    store.append(records.clone()).unwrap();
    let index = LexicalIndex::new(&store, 0).unwrap();
    report(&format!(
        "lexical index of {count} names built in {:?} (median of 3), {:.1} MiB, limit {:?}",
        builds[1],
        index.heap_bytes() as f64 / f64::from(1 << 20),
        limit("max_build_ms")
    ));
    assert!(builds[1] <= limit("max_build_ms"));

    let queries = 200;
    let budgeted = budget("threshold").as_float().unwrap();
    for threshold in [0.7, budgeted, 0.9] {
        let mut samples = Vec::with_capacity(queries);
        let mut verified = 0;
        for query in records.iter().step_by(count / queries).take(queries) {
            let start = Instant::now();
            #[allow(clippy::cast_possible_truncation)]
            let result = index
                .search(&store, query, threshold as f32, usize::MAX)
                .unwrap();
            samples.push(start.elapsed());
            assert!(result.matches.iter().any(|found| found.similarity >= 1.0));
            verified += result.candidates_verified;
        }
        samples.sort_unstable();
        let (median, p95) = (samples[queries / 2], samples[queries * 95 / 100]);
        let budgeted_threshold = (threshold - budgeted).abs() < 1e-9;
        report(&format!(
            "lexical search at threshold {threshold} over {count} names: median {median:?}, p95 {p95:?}, {} candidates verified per query on average ({queries} queries, one thread){}",
            verified / queries,
            if budgeted_threshold {
                format!(", limit {:?}", limit("max_query_ms"))
            } else {
                String::new()
            }
        ));
        if budgeted_threshold {
            assert!(p95 <= limit("max_query_ms"));
        }
    }
}
