//! Resource budget for exhaustive vector search, from
//! `benchmarks/budgets.toml`, on synthetic unit vectors of the default
//! model's dimension. Release builds only; shared CI runners multiply the
//! time budget by `UNIVO_PERF_SLACK`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use univo_core::{FieldPath, Map, Schema, Text, TextMode, Value, prepare};
use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};
use univo_engine::{embed_records, top_k};

const DIMENSION: usize = 128;

/// A deterministic pseudo-random unit vector per text.
struct RandomEmbedder(ModelDescriptor);

impl Embedder for RandomEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.0
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        let mut data = Vec::with_capacity(texts.len() * DIMENSION);
        for text in texts {
            let mut state: u64 = text.parse::<u64>().unwrap() * 0x9E37_79B9_7F4A_7C15 + 1;
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
    budgets["engine"]["vector_search"][key].clone()
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

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "performance budgets apply to release builds"
)]
fn exhaustive_vector_search_stays_within_budget() {
    let _alone = univo_testkit::timing::exclusive();
    let records = usize::try_from(budget("records").as_integer().unwrap()).unwrap();
    let k = usize::try_from(budget("k").as_integer().unwrap()).unwrap();
    let slack: f64 = std::env::var("UNIVO_PERF_SLACK").map_or(1.0, |value| value.parse().unwrap());
    assert!(slack.is_finite() && slack >= 1.0);
    let limit = Duration::from_secs_f64(budget("max_ms").as_float().unwrap() / 1000.0 * slack);

    let schema = Schema::builder()
        .text(
            FieldPath::parse("text").unwrap(),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .build()
        .unwrap();
    let prepared: Vec<_> = (0..records)
        .map(|index| {
            let value = Map::new(vec![("text".into(), Value::Str(index.to_string()))]).unwrap();
            prepare(&schema, &Value::Map(value)).unwrap()
        })
        .collect();
    let embedder = RandomEmbedder(ModelDescriptor {
        source: ModelSource::Local {
            path: "random".into(),
            files: Vec::new(),
        },
        dimension: DIMENSION,
        backend: "random",
    });
    let vectors = embed_records(&schema, &prepared, &embedder).unwrap();
    let field = vectors.field(0).unwrap();

    let queries = 25;
    let mut samples: Vec<Duration> = (0..=queries)
        .map(|query| {
            let vector = field.get(query * 997 % records).unwrap();
            let start = Instant::now();
            let found = top_k(field, vector, 0..records, k, |slot| slot);
            let elapsed = start.elapsed();
            assert_eq!(found.len(), k);
            assert_eq!(found[0].slot, query * 997 % records);
            elapsed
        })
        .skip(1)
        .collect();
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    #[allow(clippy::cast_precision_loss)]
    let rate = (records * DIMENSION) as f64 / median.as_secs_f64() / 1e9;
    report(&format!(
        "exhaustive top-{k} over {records} vectors of {DIMENSION} dimensions: median {median:?} (range {:?}–{:?}, {queries} queries, one thread), {rate:.2} G multiply-adds per second, limit {limit:?}",
        samples[0],
        samples[samples.len() - 1],
    ));
    assert!(median <= limit, "median {median:?} exceeded {limit:?}");
    assert!(vectors.heap_bytes() >= records * DIMENSION * 4);
}
