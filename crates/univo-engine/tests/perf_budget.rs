use std::path::PathBuf;
use std::time::{Duration, Instant};

use univo_core::{FieldPath, Map, PreparedRecord, Schema, Text, Value, prepare};
use univo_engine::ExactIndex;

fn workload(count: usize, identified: bool) -> (Schema, Vec<PreparedRecord>) {
    let schema = if identified {
        Schema::builder()
            .id_field(FieldPath::parse("id").unwrap())
            .text(FieldPath::parse("name").unwrap(), Text::default())
            .build()
            .unwrap()
    } else {
        Schema::strings()
    };
    let records = (0..count)
        .rev()
        .map(|id| {
            let value = if identified {
                Value::Map(
                    Map::new(vec![
                        ("id".into(), Value::Int(i64::try_from(id).unwrap())),
                        ("name".into(), Value::Str("same".into())),
                    ])
                    .unwrap(),
                )
            } else {
                Value::Str("same".into())
            };
            prepare(&schema, &value).unwrap()
        })
        .collect();
    (schema, records)
}

fn measure(count: usize, identified: bool, incremental: bool) -> Vec<Duration> {
    let mut samples = Vec::new();
    for run in 0..6 {
        let (schema, records) = workload(count, identified);
        let mut index = ExactIndex::new(schema, count);
        let start = Instant::now();
        if incremental {
            for record in records {
                index.add(vec![record]).unwrap();
            }
        } else {
            index.add(records).unwrap();
        }
        let elapsed = start.elapsed();
        if run > 0 {
            samples.push(elapsed);
        }
        assert_eq!(std::hint::black_box(index.len()), count);
        let query = index.record(0).unwrap();
        let result = index.search(query, 3).unwrap();
        assert_eq!(result.total_matches, count - usize::from(identified));
        let expected: Vec<_> = if identified {
            (count - 3..count).rev().collect()
        } else {
            (0..3).collect()
        };
        assert_eq!(
            result
                .matches
                .iter()
                .map(|hit| hit.slot)
                .collect::<Vec<_>>(),
            expected
        );
    }
    samples.sort_unstable();
    samples
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
fn dense_incremental_insertion_stays_within_budget() {
    let _alone = univo_testkit::timing::exclusive();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/budgets.toml");
    let budgets: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
    let budget = &budgets["engine"]["dense_insert"];
    let records = usize::try_from(budget["records"].as_integer().unwrap()).unwrap();
    let slack: f64 = std::env::var("UNIVO_PERF_SLACK").map_or(1.0, |value| value.parse().unwrap());
    assert!(slack.is_finite() && slack >= 1.0);
    let limit = Duration::from_secs_f64(budget["max_ms"].as_float().unwrap() / 1000.0 * slack);
    for count in [records / 4, records / 2, records] {
        for identified in [false, true] {
            let batch = measure(count, identified, false);
            let incremental = measure(count, identified, true);
            report(&format!(
                "dense insertion, {count} records, IDs={identified}: batch {:?} (range {:?}–{:?}), one at a time {:?} (range {:?}–{:?}), limit {limit:?} (median of 5 after warmup, one thread, preparation excluded)",
                batch[2], batch[0], batch[4], incremental[2], incremental[0], incremental[4],
            ));
            assert!(batch[2] <= limit, "batch insertion exceeded {limit:?}");
            assert!(
                incremental[2] <= limit,
                "incremental insertion exceeded {limit:?}"
            );
        }
    }
}
