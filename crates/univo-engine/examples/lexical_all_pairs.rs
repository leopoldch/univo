//! All-pairs `find` over one partition of lexical names read from a file,
//! one per line, at minimum score 0.8 on a given number of threads: the
//! cost of the lexical lane and its bounds (`docs/design/lexical-search.md`).
//! Prints the time, the matches and a hash of the whole report, so that
//! two builds can be checked for identical results.
//!
//! `cargo run --release -p univo-engine --example lexical_all_pairs -- names.txt 100000 8`

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;

use univo_core::{FieldPath, Map, Schema, Text, TextMode, Value, prepare};
use univo_engine::{SearchIndex, SearchOptions};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a file of names, one per line");
    let records: usize = args.next().map_or(100_000, |n| n.parse().unwrap());
    let threads: usize = args.next().map_or(8, |n| n.parse().unwrap());

    let schema = Schema::builder()
        .id_field(FieldPath::parse("row").unwrap())
        .text(
            FieldPath::parse("name").unwrap(),
            Text {
                mode: TextMode::Lexical,
                ..Text::default()
            },
        )
        .build()
        .unwrap();
    let names = std::fs::read_to_string(&path).unwrap();
    let prepared = names
        .lines()
        .take(records)
        .zip(0i64..)
        .map(|(name, row)| {
            let value = Map::new(vec![
                ("row".into(), Value::Int(row)),
                ("name".into(), Value::Str(name.into())),
            ])
            .unwrap();
            prepare(&schema, &Value::Map(value)).unwrap()
        })
        .collect::<Vec<_>>();
    let count = prepared.len();
    let mut index = SearchIndex::new(schema, count, None).unwrap();
    index.add(prepared).unwrap();

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    let options = SearchOptions {
        min_score: 0.8,
        limit: usize::MAX,
        max_entries: usize::MAX,
    };
    let start = Instant::now();
    let report = pool
        .install(|| index.find(options, false, usize::MAX, usize::MAX))
        .unwrap();
    let elapsed = start.elapsed();
    let mut hash = DefaultHasher::new();
    format!("{report:?}").hash(&mut hash);
    println!(
        "{count} names, {threads} threads: {elapsed:.2?}, {} matches, truncated {}, report {:016x}",
        report.total_matches,
        report.candidate_truncated,
        hash.finish()
    );
}
