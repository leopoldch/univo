//! End-to-end search budget, from `benchmarks/budgets.toml`: the standard
//! workload of specs/07-performance.md (ID, tenant, a name of about 20
//! characters, a semantic description) with synthetic embeddings of the
//! default model's dimension, following its protocol: five independent
//! repetitions of 1000 queries after a warmup. Release builds only; shared
//! CI runners multiply time budgets by `UNIVO_PERF_SLACK`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use univo_core::{FieldPath, Map, PreparedRecord, Schema, Text, TextMode, Value, prepare};
use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};
use univo_engine::{SearchIndex, SearchOptions};

const DIMENSION: usize = 256;

const SYLLABLES: &[&str] = &[
    "ka", "lo", "mi", "ne", "ra", "to", "vi", "su", "dor", "pel", "ban", "tri", "gon", "zu", "fa",
    "re", "qui", "mo", "sel", "an", "ex", "or", "il", "ty", "cha", "vel", "bro", "kin", "dal",
    "mer",
];

/// A deterministic pseudo-random unit vector per text: unrelated texts are
/// nearly orthogonal, as with a real model on diverse descriptions.
struct RandomEmbedder(ModelDescriptor);

impl Embedder for RandomEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.0
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        let mut data = Vec::with_capacity(texts.len() * DIMENSION);
        for text in texts {
            let mut state = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
            });
            let row: Vec<f32> = (0..DIMENSION)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    f32::from(u16::try_from(state % 2001).unwrap()) / 1000.0 - 1.0
                })
                .collect();
            let norm = row.iter().map(|value| value * value).sum::<f32>().sqrt();
            data.extend(row.iter().map(|value| value / norm));
        }
        Embeddings::new("random", data, DIMENSION)
    }
}

fn budget(key: &str) -> toml::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/budgets.toml");
    let budgets: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
    budgets["engine"]["search"][key].clone()
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

/// `count` records in `tenants` partitions; the first `group` are copies of
/// one record, with their own IDs.
fn workload(schema: &Schema, count: usize, tenants: usize, group: usize) -> Vec<PreparedRecord> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move |bound: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        usize::try_from(state % bound as u64).unwrap()
    };
    let word = |syllables: usize, next: &mut dyn FnMut(usize) -> usize| -> String {
        (0..syllables)
            .map(|_| SYLLABLES[next(SYLLABLES.len())])
            .collect()
    };
    let mut previous: Option<(String, String, String)> = None;
    (0..count)
        .map(|id| {
            // One record in ten repeats the previous one with a typo in its
            // name: a duplicate to find.
            let (tenant, name, description) = match previous.take() {
                Some(copy) if id < group => copy,
                Some((tenant, mut name, description)) if next(10) == 0 => {
                    name.replace_range(0..1, "x");
                    (tenant, name, description)
                }
                _ => {
                    let words = 2 + next(2);
                    let name: Vec<String> =
                        (0..words).map(|_| word(2 + next(2), &mut next)).collect();
                    let description: Vec<String> = (0..30).map(|_| word(2, &mut next)).collect();
                    (
                        format!("tenant-{}", next(tenants)),
                        format!("{} {}", name.join(" "), next(1000)),
                        description.join(" "),
                    )
                }
            };
            previous = Some((tenant.clone(), name.clone(), description.clone()));
            let entries = vec![
                ("id".to_owned(), Value::Int(i64::try_from(id).unwrap())),
                ("tenant".to_owned(), Value::Str(tenant)),
                ("name".to_owned(), Value::Str(name)),
                ("description".to_owned(), Value::Str(description)),
            ];
            prepare(schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
        })
        .collect()
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
#[cfg_attr(
    debug_assertions,
    ignore = "performance budgets apply to release builds"
)]
fn search_stays_within_budget() {
    let count = usize::try_from(budget("records").as_integer().unwrap()).unwrap();
    let min_score = budget("min_score").as_float().unwrap() as f32;
    let slack: f64 = std::env::var("UNIVO_PERF_SLACK").map_or(1.0, |value| value.parse().unwrap());
    assert!(slack.is_finite() && slack >= 1.0);
    let path = |dotted| FieldPath::parse(dotted).unwrap();
    let schema = Schema::builder()
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
        .build()
        .unwrap();
    let embedder: Arc<dyn Embedder> = Arc::new(RandomEmbedder(ModelDescriptor {
        source: ModelSource::Local {
            path: "random".into(),
            files: Vec::new(),
        },
        dimension: DIMENSION,
        backend: "random",
    }));

    // Scenarios: one partition, the worst case; ten partitions; and one
    // partition holding a group of identical records, queried from it.
    let group = count / 10;
    for (label, tenants, group, budget_key) in [
        ("1 partition", 1, 0, "max_ms"),
        ("10 partitions", 10, 0, "max_ms"),
        (
            "1 partition with a group of identical records",
            1,
            group,
            "max_group_ms",
        ),
    ] {
        let limit =
            Duration::from_secs_f64(budget(budget_key).as_float().unwrap() / 1000.0 * slack);
        let options = SearchOptions {
            min_score,
            limit: 10,
            max_entries: usize::MAX,
        };
        let queries = 1000;
        let slots: Vec<usize> = if group > 0 {
            (0..group).step_by(group / queries).take(queries).collect()
        } else {
            (0..count).step_by(count / queries).take(queries).collect()
        };
        let mut repetitions = Vec::new();
        let (mut build, mut heap, mut candidates, mut found) = (Duration::ZERO, 0, 0, 0);
        for _ in 0..5 {
            let records = workload(&schema, count, tenants, group);
            let mut index =
                SearchIndex::new(schema.clone(), count, Some(embedder.clone())).unwrap();
            let start = Instant::now();
            index.add(records).unwrap();
            build = start.elapsed();
            heap = index.heap_bytes();
            for &slot in slots.iter().take(50) {
                std::hint::black_box(index.search_slot(slot, options).unwrap());
            }
            let mut samples = Vec::with_capacity(queries);
            (candidates, found) = (0, 0);
            for &slot in &slots {
                let start = Instant::now();
                let result = index.search_slot(slot, options).unwrap();
                samples.push(start.elapsed());
                assert!(!result.truncated);
                assert!(result.matches.len() <= options.limit);
                candidates += result.candidates;
                found += result.total_matches;
            }
            samples.sort_unstable();
            repetitions.push((
                samples[queries / 2],
                samples[queries * 95 / 100],
                samples[queries * 99 / 100],
            ));
        }
        assert!(found > 0, "the workload must contain duplicates");
        repetitions.sort_unstable_by_key(|(_, p95, _)| *p95);
        let (median, p95, p99) = repetitions[2];
        report(&format!(
            "search of {count} records, {label}, minimum score {min_score}, limit 10: median {median:?}, p95 {p95:?} (median of 5 repetitions, range {:?}–{:?}), p99 {p99:?}; {:.1} records scored and {:.1} matches per query ({queries} queries, one thread); build {build:?} with synthetic embeddings, indexes {:.0} MiB; limit {limit:?}",
            repetitions[0].1,
            repetitions[4].1,
            candidates as f64 / queries as f64,
            found as f64 / queries as f64,
            heap as f64 / f64::from(1 << 20),
        ));
        assert!(p95 <= limit, "p95 {p95:?} exceeded {limit:?}");
    }
}
