//! Resource budgets for the default model, from `benchmarks/budgets.toml`.
//!
//! These tests need the default model's files: they run only when
//! `UNIVO_TEST_MODEL_DIR` names a directory holding them, and verify the
//! files against the pinned checksums first. Budgets apply to release builds
//! on the reference machine described in `docs/development/performance.md`; shared CI
//! runners multiply time budgets by `UNIVO_PERF_SLACK`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use univo_embed::{Embedder, StaticEmbedder};

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
    samples.sort();
    samples[samples.len() / 2]
}

/// The verified default model, or `None` when it is not available locally.
fn default_model_dir() -> Option<PathBuf> {
    let directory = PathBuf::from(std::env::var_os("UNIVO_TEST_MODEL_DIR")?);
    for file in &univo_embed::registry::default_model().files {
        univo_embed::files::verify_file("default", &directory.join(&file.name), file).unwrap();
    }
    Some(directory)
}

/// Descriptions of the standard workload: about
/// 30 tokens each, from a deterministic xorshift generator.
fn descriptions(count: usize) -> Vec<String> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        usize::try_from(state % 1_000_003).unwrap()
    };
    (0..count)
        .map(|_| {
            (0..30)
                .map(|_| WORDS[next() % WORDS.len()])
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "performance budgets apply to release builds"
)]
fn loading_and_first_encoding_stay_within_budget() {
    let _alone = univo_testkit::timing::exclusive();
    let Some(directory) = default_model_dir() else {
        eprintln!("skipped: UNIVO_TEST_MODEL_DIR is not set");
        return;
    };
    let samples = (0..5)
        .map(|_| {
            let start = Instant::now();
            let embedder = StaticEmbedder::load_directory(&directory).unwrap();
            embedder.encode_batch(&["Amazon.com Inc."]).unwrap();
            start.elapsed()
        })
        .collect();
    let elapsed = median(samples);
    let limit = Duration::from_secs_f64(
        budget("embed.first_encoding", "max_ms").as_float().unwrap() * time_slack() / 1000.0,
    );
    report(&format!(
        "loaded the default model and encoded one text in {elapsed:?} (median of 5, files in the OS cache), limit {limit:?}"
    ));
    assert!(elapsed <= limit);
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "performance budgets apply to release builds"
)]
fn encoding_the_standard_workload_stays_within_budget() {
    let _alone = univo_testkit::timing::exclusive();
    let Some(directory) = default_model_dir() else {
        eprintln!("skipped: UNIVO_TEST_MODEL_DIR is not set");
        return;
    };
    let embedder = StaticEmbedder::load_directory(&directory).unwrap();
    let count = usize::try_from(budget("embed.encode", "texts").as_integer().unwrap()).unwrap();
    let texts = descriptions(count);
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();

    let single = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let measure = |pool: Option<&rayon::ThreadPool>| {
        let run = || {
            let start = Instant::now();
            let embeddings = embedder.encode_batch(&texts).unwrap();
            assert_eq!(embeddings.len(), count);
            start.elapsed()
        };
        let _warmup = run();
        median(
            (0..5)
                .map(|_| pool.map_or_else(run, |pool| pool.install(run)))
                .collect(),
        )
    };
    let one_thread = measure(Some(&single));
    let all_threads = measure(None);

    let limit = Duration::from_secs_f64(
        budget("embed.encode", "max_ms").as_float().unwrap() * time_slack() / 1000.0,
    );
    report(&format!(
        "encoded {count} descriptions in {one_thread:?} on one thread, {all_threads:?} on {} threads (median of 5), {:.0} ns per text on one thread, limit {limit:?}",
        rayon::current_num_threads(),
        one_thread.as_secs_f64() * 1e9 / f64::from(u32::try_from(count).unwrap())
    ));
    assert!(one_thread <= limit);
}
