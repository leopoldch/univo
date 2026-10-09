//! End-to-end search budget, from `benchmarks/budgets.toml`: the standard
//! workload of specs/07-performance.md (ID, tenant, a name of about 20
//! characters, a semantic description) with synthetic embeddings of the
//! default model's dimension. Release builds only; shared CI runners
//! multiply time budgets by `UNIVO_PERF_SLACK`.

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

fn workload(schema: &Schema, count: usize, tenants: usize) -> Vec<PreparedRecord> {
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
    let limit = Duration::from_secs_f64(budget("max_ms").as_float().unwrap() / 1000.0 * slack);
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

    for tenants in [1, 10] {
        let records = workload(&schema, count, tenants);
        let mut index = SearchIndex::new(schema.clone(), count, Some(embedder.clone())).unwrap();
        let start = Instant::now();
        index.add(records).unwrap();
        let build = start.elapsed();

        let options = SearchOptions {
            min_score,
            limit: 10,
            max_entries: usize::MAX,
        };
        let queries = 200;
        let mut samples = Vec::with_capacity(queries);
        let (mut candidates, mut found) = (0, 0);
        for slot in (0..count).step_by(count / queries).take(queries) {
            let start = Instant::now();
            let result = index.search_slot(slot, options).unwrap();
            samples.push(start.elapsed());
            assert!(!result.truncated);
            candidates += result.candidates;
            found += result.total_matches;
        }
        assert!(found > 0, "the workload must contain duplicates");
        samples.sort_unstable();
        let (median, p95) = (samples[queries / 2], samples[queries * 95 / 100]);
        report(&format!(
            "search of {count} records in {tenants} partition(s) at minimum score {min_score}: median {median:?}, p95 {p95:?}, {:.1} records scored per query, {found} matches in all ({queries} queries, one thread); build {build:?} with synthetic embeddings, indexes {:.0} MiB; limit {limit:?}",
            candidates as f64 / queries as f64,
            index.heap_bytes() as f64 / f64::from(1 << 20),
        ));
        assert!(p95 <= limit, "p95 {p95:?} exceeded {limit:?}");
    }
}
