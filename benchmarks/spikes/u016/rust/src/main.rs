mod distance;
use distance::Euclidean;
use fast_hnsw::{Builder, hnsw::PruneStrategy};
use hnsw_rs::prelude::{DistL2, Hnsw};
use serde_json::json;
use std::time::Instant;

fn random(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    ((*state >> 40) as u32 as f32) / 16_777_216.0 - 0.5
}
fn vectors(n: usize, dimension: usize, clustered: bool, seed: u64) -> Vec<Vec<f32>> {
    let mut state = seed;
    let centers: Vec<Vec<f32>> = (0..32)
        .map(|_| (0..dimension).map(|_| random(&mut state)).collect())
        .collect();
    (0..n)
        .map(|i| {
            let mut row: Vec<f32> = (0..dimension)
                .map(|d| {
                    random(&mut state)
                        + if clustered {
                            centers[i % 32][d] * 3.0
                        } else {
                            0.0
                        }
                })
                .collect();
            let norm = row.iter().map(|v| v * v).sum::<f32>().sqrt();
            for value in &mut row {
                *value /= norm;
            }
            row
        })
        .collect()
}
#[derive(PartialEq)]
struct Ranked(f32, usize);
impl Eq for Ranked {}
impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0).then(self.1.cmp(&other.1))
    }
}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
fn nearest(points: &[Vec<f32>], q: &[f32]) -> Vec<usize> {
    use fast_hnsw::distance::Distance;
    let mut kept = std::collections::BinaryHeap::with_capacity(11);
    for (id, point) in points.iter().enumerate() {
        let candidate = Ranked(Euclidean.distance(point, q), id);
        if kept.len() < 10 {
            kept.push(candidate);
        } else if kept.peek().is_some_and(|worst| candidate < *worst) {
            kept.pop();
            kept.push(candidate);
        }
    }
    kept.into_sorted_vec().into_iter().map(|v| v.1).collect()
}
fn read_vectors(path: &str) -> Vec<Vec<f32>> {
    let bytes = std::fs::read(path).unwrap();
    let n = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    let dimension = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 16 + n * dimension * 4);
    bytes[16..]
        .chunks_exact(dimension * 4)
        .map(|row| {
            row.chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect()
        })
        .collect()
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let backend = &args[1];
    let n: usize = args[2].parse().unwrap();
    let dimension: usize = args[3].parse().unwrap();
    let clustered = args[4] == "clustered";
    let points = if args.len() > 5 {
        read_vectors(&args[5])
    } else {
        vectors(n, dimension, clustered, 41)
    };
    let n = points.len();
    // Held-out queries; centered near points, but never inserted.
    let queries: Vec<Vec<f32>> = if args.len() > 6 {
        read_vectors(&args[6])
    } else {
        points
            .iter()
            .step_by((n / 200).max(1))
            .take(200)
            .enumerate()
            .map(|(i, point)| {
                let noise = vectors(1, dimension, false, 1000 + i as u64).pop().unwrap();
                let mut row: Vec<_> = point
                    .iter()
                    .zip(noise)
                    .map(|(v, noise)| v + noise * 0.05)
                    .collect();
                let norm = row.iter().map(|v| v * v).sum::<f32>().sqrt();
                for v in &mut row {
                    *v /= norm;
                }
                row
            })
            .collect()
    };
    let start = Instant::now();
    let oracle: Vec<_> = queries.iter().map(|q| nearest(&points, q)).collect();
    let exact_ms = start.elapsed().as_secs_f64() * 1000.0 / queries.len() as f64;
    let start = Instant::now();
    let mut fast = None;
    let mut reference = None;
    if backend.starts_with("fast") {
        let mut index = Builder::new()
            .m(16)
            .ef_construction(200)
            .prune_strategy(if backend == "fast-simple" {
                PruneStrategy::Simple
            } else {
                PruneStrategy::Heuristic
            })
            .seed(16)
            .capacity(n)
            .build(Euclidean);
        for point in &points {
            index.insert(point.clone());
        }
        fast = Some(index);
    } else {
        let index = Hnsw::<f32, DistL2>::new(16, n, 16, 200, DistL2 {});
        for (i, point) in points.iter().enumerate() {
            index.insert((point, i));
        }
        reference = Some(index);
    }
    let build_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut results = Vec::new();
    for ef in [32, 128, 512, 1024] {
        let mut latencies = Vec::new();
        let mut matched = 0;
        for (q, expected) in queries.iter().zip(&oracle) {
            let start = Instant::now();
            let found: Vec<_> = match &fast {
                Some(index) => index.search(q, 10, ef).iter().map(|v| v.id).collect(),
                None => reference
                    .as_ref()
                    .unwrap()
                    .search(q, 10, ef)
                    .iter()
                    .map(|v| v.d_id)
                    .collect(),
            };
            latencies.push(start.elapsed().as_secs_f64() * 1000.0);
            matched += found.iter().filter(|id| expected.contains(id)).count();
        }
        latencies.sort_by(f64::total_cmp);
        results.push(json!({"ef":ef,"recall_at_10":matched as f64/(queries.len()*10) as f64,"p50_ms":latencies[latencies.len()/2],"p95_ms":latencies[latencies.len()*95/100]}));
    }
    println!(
        "{}",
        json!({"backend":backend,"records":n,"dimension":dimension,"clustered":clustered,"queries":queries.len(),"m":16,"ef_construction":200,"exact_mean_ms":exact_ms,"build_ms":build_ms,"search":results})
    );
}
