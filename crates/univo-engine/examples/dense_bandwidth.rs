//! Dense scan throughput with the vectors in cache or in memory, on 1 to 8
//! threads: each query scans every row with the scoring cosine. Separates
//! the cost of arithmetic from the cost of reading vectors (ADR 0018).
//!
//! `cargo run --release -p univo-engine --example dense_bandwidth`

use std::time::Instant;

use rayon::prelude::*;

const DIMENSION: usize = 256;

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
    println!("vectors_mb,threads,ns_per_dot,read_gb_per_s");
    // 1 MB fits one core's cache; 100 MB is a 100,000-record partition.
    for (rows, queries) in [(1_000usize, 64_000usize), (100_000, 640)] {
        let data: Vec<f32> = (0..rows * DIMENSION).map(|_| next()).collect();
        for threads in [1, 2, 4, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .expect("a worker pool");
            let mut best = f64::MAX;
            for _ in 0..3 {
                let start = Instant::now();
                let hits: usize = pool.install(|| {
                    (0..queries)
                        .into_par_iter()
                        .map(|query| {
                            let query = &data[(query % rows) * DIMENSION..][..DIMENSION];
                            data.as_chunks::<DIMENSION>()
                                .0
                                .iter()
                                .filter(|row| {
                                    univo_core::semantic_similarity(query, row.as_slice()) >= 0.8
                                })
                                .count()
                        })
                        .sum()
                });
                std::hint::black_box(hits);
                best = best.min(start.elapsed().as_secs_f64());
            }
            #[allow(clippy::cast_precision_loss)]
            let (dots, dimension) = ((rows * queries) as f64, DIMENSION as f64);
            println!(
                "{},{threads},{:.2},{:.1}",
                rows * DIMENSION * 4 / 1_000_000,
                best * 1e9 / dots,
                dots * 4.0 * dimension / best / 1e9,
            );
        }
    }
}
