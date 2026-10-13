//! Dense scan throughput with the vectors in cache or in memory, on 1 to 8
//! threads, query by query or in blocks of queries sharing each row read.
//! Separates the cost of arithmetic from the cost of reading vectors
//! (ADR 0018, ADR 0019).
//!
//! `cargo run --release -p univo-engine --example dense_bandwidth`

use std::time::Instant;

use rayon::prelude::*;

const DIMENSION: usize = 256;
const BLOCK: usize = 64;

/// Matches of one query, scanning every row.
fn single(data: &[f32], query: &[f32]) -> usize {
    data.as_chunks::<DIMENSION>()
        .0
        .iter()
        .filter(|row| univo_core::semantic_similarity(query, row.as_slice()) >= 0.8)
        .count()
}

/// Matches of a block of queries, reading each row once for the block.
fn blocked(data: &[f32], queries: &[&[f32]]) -> usize {
    let mut hits = 0;
    for row in data.as_chunks::<DIMENSION>().0 {
        let (quads, rest) = queries.as_chunks::<4>();
        for quad in quads {
            let found = univo_core::dots(*quad, row.as_slice());
            hits += found.iter().filter(|&&dot| dot >= 0.8).count();
        }
        for query in rest {
            hits += usize::from(univo_core::dot(query, row.as_slice()) >= 0.8);
        }
    }
    hits
}

fn main() {
    let mut state = 7u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        #[allow(clippy::cast_precision_loss)]
        let value = (state % 2000) as f32 / 1000.0 - 1.0;
        value
    };
    println!("vectors_mb,mode,threads,ns_per_dot,read_gb_per_s");
    // 1 MB fits one core's cache; 100 MB is a 100,000-record partition.
    for (rows, queries) in [(1_000usize, 64_000usize), (100_000, 1_024)] {
        let data: Vec<f32> = (0..rows * DIMENSION).map(|_| next()).collect();
        let query = |index: usize| &data[(index % rows) * DIMENSION..][..DIMENSION];
        for mode in ["single", "blocked"] {
            for threads in [1, 2, 4, 8] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .expect("a worker pool");
                let mut best = f64::MAX;
                for _ in 0..3 {
                    let start = Instant::now();
                    let hits: usize = pool.install(|| match mode {
                        "single" => (0..queries)
                            .into_par_iter()
                            .map(|index| single(&data, query(index)))
                            .sum(),
                        _ => (0..queries / BLOCK)
                            .into_par_iter()
                            .map(|block| {
                                let block: Vec<&[f32]> =
                                    (block * BLOCK..(block + 1) * BLOCK).map(query).collect();
                                blocked(&data, &block)
                            })
                            .sum(),
                    });
                    std::hint::black_box(hits);
                    best = best.min(start.elapsed().as_secs_f64());
                }
                #[allow(clippy::cast_precision_loss)]
                let (dots, dimension, block) =
                    ((rows * queries) as f64, DIMENSION as f64, BLOCK as f64);
                let reads = if mode == "single" { dots } else { dots / block };
                println!(
                    "{},{mode},{threads},{:.2},{:.1}",
                    rows * DIMENSION * 4 / 1_000_000,
                    best * 1e9 / dots,
                    reads * 4.0 * dimension / best / 1e9,
                );
            }
        }
    }
}
