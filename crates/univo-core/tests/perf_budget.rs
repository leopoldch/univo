//! Resource budgets for record preparation, from `benchmarks/budgets.toml`.
//!
//! Budgets apply to release builds on the reference machine described in
//! `docs/performance.md`; shared CI runners multiply time budgets by
//! `UNIVO_PERF_SLACK`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use univo_core::{
    Exact, ExactNormalizer, FieldPath, Map, Schema, Text, TextMode, Value, compare, prepare,
};

const WORDS: &[&str] = &[
    "online",
    "retail",
    "cloud",
    "services",
    "platform",
    "payments",
    "logistics",
    "software",
    "consulting",
    "energy",
    "Société",
    "française",
    "spécialisée",
    "distribution",
    "hypermarchés",
    "électronique",
    "européenne",
    "Zürich",
    "Straße",
    "café",
    "data",
    "analytics",
    "security",
];

fn budget(section: &str, key: &str) -> toml::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/budgets.toml");
    let budgets: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
    let mut table = &budgets;
    for part in section.split('.') {
        table = table[part].as_table().unwrap();
    }
    table[key].clone()
}

fn time_slack() -> f64 {
    let slack: f64 = std::env::var("UNIVO_PERF_SLACK").map_or(1.0, |value| value.parse().unwrap());
    assert!(slack >= 1.0, "UNIVO_PERF_SLACK must be >= 1.0");
    slack
}

/// Deterministic xorshift generator, so every run measures the same input.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % 1_000_003).unwrap()
    }

    fn sentence(&mut self, words: usize) -> String {
        (0..words)
            .map(|_| WORDS[self.next() % WORDS.len()])
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The standard workload of specs/07-performance.md: identifier, tenant, a
/// name of about 20 characters and a description of about 30 tokens. One
/// record in 20 is in decomposed Unicode form.
fn workload(records: usize, tenants: u64) -> Vec<Value> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    (0..records)
        .map(|index| {
            let mut name = rng.sentence(2);
            name.truncate(name.char_indices().nth(20).map_or(name.len(), |(at, _)| at));
            let mut description = rng.sentence(30);
            if index.is_multiple_of(20) {
                description = description.replace('é', "e\u{301}");
            }
            let entries = vec![
                ("id".to_owned(), Value::Str(format!("record-{index}"))),
                (
                    "tenant_id".to_owned(),
                    Value::Str(format!("tenant-{}", rng.next() as u64 % tenants)),
                ),
                ("name".to_owned(), Value::Str(name)),
                (
                    "domain".to_owned(),
                    Value::Str(format!("Company{index}.COM")),
                ),
                ("description".to_owned(), Value::Str(description)),
            ];
            Value::Map(Map::new(entries).unwrap())
        })
        .collect()
}

fn schema() -> Schema {
    let path = |dotted| FieldPath::parse(dotted).unwrap();
    Schema::builder()
        .id_field(path("id"))
        .partition_by(path("tenant_id"))
        .text(path("name"), Text::default())
        .exact(
            path("domain"),
            Exact {
                normalizer: ExactNormalizer::Hostname,
                ..Exact::default()
            },
        )
        .text(
            path("description"),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .build()
        .unwrap()
}

/// Prints a measurement, and records it in the job summary on GitHub
/// Actions so that every platform's numbers are kept, not only failures.
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

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "performance budgets apply to release builds"
)]
fn preparing_the_standard_workload_stays_within_budget() {
    let records = budget("core.prepare", "records").as_integer().unwrap();
    let records = usize::try_from(records).unwrap();
    let values = workload(records, 50);
    let schema = schema();

    let mut samples = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let prepared: Vec<_> = values
            .iter()
            .map(|value| prepare(&schema, value).unwrap())
            .collect();
        samples.push(start.elapsed());
        assert_eq!(std::hint::black_box(prepared).len(), records);
    }

    let elapsed = median(samples);
    let limit = Duration::from_secs_f64(
        budget("core.prepare", "max_ms").as_float().unwrap() / 1000.0 * time_slack(),
    );
    report(&format!(
        "prepared {records} records in {elapsed:?} (median of 5), {:.0} ns per record, limit {limit:?}",
        elapsed.as_secs_f64() * 1e9 / f64::from(u32::try_from(records).unwrap())
    ));
    assert!(
        elapsed <= limit,
        "preparation took {elapsed:?}, above {limit:?}"
    );
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "performance budgets apply to release builds"
)]
fn comparing_candidate_pairs_stays_within_budget() {
    let pairs = budget("core.compare", "pairs").as_integer().unwrap();
    let pairs = usize::try_from(pairs).unwrap();
    let candidates =
        usize::try_from(budget("core.compare", "candidates").as_integer().unwrap()).unwrap();
    let schema = schema();
    // Candidates share a partition: records of different partitions are never
    // compared, and would make the measurement trivially fast.
    let records: Vec<_> = workload(pairs / candidates + candidates, 1)
        .iter()
        .map(|value| prepare(&schema, value).unwrap())
        .collect();

    let mut samples = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let mut compared = 0usize;
        for (index, record) in records.iter().take(pairs / candidates).enumerate() {
            for other in &records[index + 1..=index + candidates] {
                std::hint::black_box(compare(&schema, record, other).unwrap());
                compared += 1;
            }
        }
        samples.push(start.elapsed());
        assert_eq!(compared, pairs);
    }

    let elapsed = median(samples);
    let limit = Duration::from_secs_f64(
        budget("core.compare", "max_ms").as_float().unwrap() / 1000.0 * time_slack(),
    );
    report(&format!(
        "compared {pairs} pairs in {elapsed:?} (median of 5), {:.0} ns per pair, limit {limit:?}",
        elapsed.as_secs_f64() * 1e9 / f64::from(u32::try_from(pairs).unwrap())
    ));
    assert!(
        elapsed <= limit,
        "comparison took {elapsed:?}, above {limit:?}"
    );
}

/// Configurations with typed parameters, one nested object, and values that
/// differ, are equal or are missing on one side, so that every branch of the
/// key-by-key comparison runs.
fn configurations(records: usize) -> Vec<Value> {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    (0..records)
        .map(|index| {
            let mut params = vec![
                (
                    "retries".to_owned(),
                    Value::Int(i64::try_from(rng.next() % 4).unwrap()),
                ),
                ("timeout_ms".to_owned(), Value::Int(1000)),
                ("tls".to_owned(), Value::Bool(rng.next().is_multiple_of(2))),
                ("region".to_owned(), Value::Str("eu-west-1".to_owned())),
                (
                    "db".to_owned(),
                    Value::Map(
                        Map::new(vec![
                            ("host".to_owned(), Value::Str("db1".to_owned())),
                            ("port".to_owned(), Value::Int(5432)),
                        ])
                        .unwrap(),
                    ),
                ),
            ];
            if index.is_multiple_of(3) {
                params.push(("batch_size".to_owned(), Value::Int(100)));
            }
            let entries = vec![
                ("id".to_owned(), Value::Str(format!("config-{index}"))),
                ("service".to_owned(), Value::Str("worker".to_owned())),
                ("environment".to_owned(), Value::Str("prod".to_owned())),
                ("params".to_owned(), Value::Map(Map::new(params).unwrap())),
                ("description".to_owned(), Value::Str(rng.sentence(12))),
            ];
            Value::Map(Map::new(entries).unwrap())
        })
        .collect()
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "performance budgets apply to release builds"
)]
fn comparing_configuration_objects_stays_within_budget() {
    let pairs = usize::try_from(
        budget("core.compare_objects", "pairs")
            .as_integer()
            .unwrap(),
    )
    .unwrap();
    let candidates = usize::try_from(
        budget("core.compare_objects", "candidates")
            .as_integer()
            .unwrap(),
    )
    .unwrap();
    let path = |dotted| FieldPath::parse(dotted).unwrap();
    let reject = Exact {
        conflict: univo_core::Conflict::Reject,
        ..Exact::default()
    };
    let schema = Schema::builder()
        .id_field(path("id"))
        .exact(path("service"), reject)
        .exact(path("environment"), reject)
        .exact(path("params"), reject)
        .text(
            path("description"),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .build()
        .unwrap();
    let records: Vec<_> = configurations(pairs / candidates + candidates)
        .iter()
        .map(|value| prepare(&schema, value).unwrap())
        .collect();

    let mut samples = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        for (index, record) in records.iter().take(pairs / candidates).enumerate() {
            for other in &records[index + 1..=index + candidates] {
                std::hint::black_box(compare(&schema, record, other).unwrap());
            }
        }
        samples.push(start.elapsed());
    }

    let elapsed = median(samples);
    let limit = Duration::from_secs_f64(
        budget("core.compare_objects", "max_ms").as_float().unwrap() / 1000.0 * time_slack(),
    );
    report(&format!(
        "compared {pairs} configuration pairs in {elapsed:?} (median of 5), {:.0} ns per pair, limit {limit:?}",
        elapsed.as_secs_f64() * 1e9 / f64::from(u32::try_from(pairs).unwrap())
    ));
    assert!(
        elapsed <= limit,
        "comparison took {elapsed:?}, above {limit:?}"
    );
}
