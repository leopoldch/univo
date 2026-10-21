//! All-pairs `find` over one partition of pseudo-random 256-dimensional
//! unit vectors, on one thread: the cost of the blocked dense lane.
//!
//! `cargo run --release -p univo-engine --example all_pairs -- 20000`

use std::sync::Arc;
use std::time::Instant;

use univo_core::{FieldPath, Map, Schema, Text, TextMode, Value, prepare};
use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};
use univo_engine::{SearchIndex, SearchOptions};

const DIMENSION: usize = 256;

/// Unit vectors drawn from a hash of the text.
#[derive(Debug)]
struct HashEmbedder(ModelDescriptor);

impl Embedder for HashEmbedder {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.0
    }

    fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
        let mut data = Vec::with_capacity(texts.len() * DIMENSION);
        for text in texts {
            let mut state = text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
            }) | 1;
            let vector: Vec<f32> = (0..DIMENSION)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    #[allow(clippy::cast_precision_loss)]
                    let value = (state % 2001) as f32 / 1000.0 - 1.0;
                    value
                })
                .collect();
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            data.extend(vector.iter().map(|x| x / norm));
        }
        Embeddings::new("hash", data, DIMENSION)
    }
}

fn main() {
    let records: usize = std::env::args()
        .nth(1)
        .map_or(20_000, |count| count.parse().expect("a record count"));
    let schema = Schema::builder()
        .text(
            FieldPath::parse("text").expect("a path"),
            Text {
                mode: TextMode::Semantic,
                ..Text::default()
            },
        )
        .build()
        .expect("a schema");
    let model: Arc<dyn Embedder> = Arc::new(HashEmbedder(ModelDescriptor {
        source: ModelSource::Local {
            path: "hash".into(),
            files: Vec::new(),
        },
        dimension: DIMENSION,
        backend: "hash",
    }));
    let prepared = (0..records)
        .map(|row| {
            let entries = vec![("text".to_owned(), Value::Str(format!("record {row}")))];
            prepare(&schema, &Value::Map(Map::new(entries).expect("a map"))).expect("a record")
        })
        .collect();
    let mut index = SearchIndex::new(schema, records, Some(model)).expect("an index");
    index.add(prepared).expect("added");
    let options = SearchOptions {
        min_score: 0.8,
        limit: 1000,
        max_entries: usize::MAX,
    };
    for _ in 0..2 {
        let start = Instant::now();
        let report = index
            .find(options, false, usize::MAX, usize::MAX)
            .expect("a report");
        let seconds = start.elapsed().as_secs_f64();
        #[allow(clippy::cast_precision_loss)]
        let pairs = (records * (records - 1) / 2) as f64;
        println!(
            "{records} records: {seconds:.2} s, {:.2} ns per pair, {} matches",
            seconds * 1e9 / pairs,
            report.total_matches
        );
    }
}
