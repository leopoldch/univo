//! Optional partition-scoped HNSW routing. Scores still come from the core.
use std::collections::HashMap;
use std::ops::Range;

use fast_hnsw::{Builder, Hnsw, hnsw::PruneStrategy};
use serde::{Deserialize, Serialize};
use univo_core::{
    Error, NormalizedValue, PreparedRecord, Result, partition_fingerprint, semantic_similarity,
};

use crate::{RecordStore, RecordVectors};

pub const BACKEND: &str = "fast-hnsw-1.0.1/euclidean8-v1";
const M: usize = 16;
const EF_CONSTRUCTION: usize = 200;
const MIN_PARTITION: usize = 10_000;
const SEED: u64 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnOptions {
    pub ef_search: usize,
    pub max_candidates: usize,
}
impl Default for AnnOptions {
    fn default() -> Self {
        Self {
            ef_search: 512,
            max_candidates: 1024,
        }
    }
}
impl AnnOptions {
    /// # Errors
    /// Refuses unbounded search/output buffers before model resolution.
    pub fn validate(self) -> Result<()> {
        if !(1..=8192).contains(&self.ef_search) {
            return Err(Error::invalid_option(
                "ann_ef_search",
                "an integer in [1, 8192]",
            ));
        }
        if !(1..=100_000).contains(&self.max_candidates) {
            return Err(Error::invalid_option(
                "ann_max_candidates",
                "an integer in [1, 100000]",
            ));
        }
        Ok(())
    }
    pub(crate) fn identity(self) -> serde_json::Value {
        serde_json::json!({"backend":BACKEND,"metric":"euclidean","m":M,"ef_construction":EF_CONSTRUCTION,"min_partition":MIN_PARTITION,"seed":SEED,"options":self})
    }
}

struct Euclidean;
impl fast_hnsw::distance::Distance for Euclidean {
    fn distance(&self, a: &[f32], b: &[f32]) -> f32 {
        let mut sums = [0.0; 8];
        let (chunks_a, rest_a) = a.as_chunks::<8>();
        let (chunks_b, rest_b) = b.as_chunks::<8>();
        for (a, b) in chunks_a.iter().zip(chunks_b) {
            for lane in 0..8 {
                let difference = a[lane] - b[lane];
                sums[lane] += difference * difference;
            }
        }
        let mut sum = sums.iter().sum::<f32>();
        for (a, b) in rest_a.iter().zip(rest_b) {
            sum += (a - b) * (a - b);
        }
        sum.sqrt()
    }
}

fn value_heap_bytes(value: &NormalizedValue) -> usize {
    match value {
        NormalizedValue::Str(text) => text.len(),
        NormalizedValue::List(values) => {
            values.len() * size_of::<NormalizedValue>()
                + values.iter().map(value_heap_bytes).sum::<usize>()
        }
        NormalizedValue::Map(values) => {
            values.len() * size_of::<(Box<str>, NormalizedValue)>()
                + values
                    .iter()
                    .map(|(key, value)| key.len() + value_heap_bytes(value))
                    .sum::<usize>()
        }
        _ => 0,
    }
}

struct Graph {
    partition: Box<[NormalizedValue]>,
    index: Hnsw<Euclidean>,
    // Bitwise vector hashes are verified; equal vectors share graph storage.
    buckets: HashMap<u64, Vec<usize>>,
    slots: Vec<Vec<u32>>,
    entries: usize,
    indexed: usize,
}
impl Graph {
    fn new(record: &PreparedRecord, indexed: usize) -> Self {
        Self {
            partition: record.partition().into(),
            index: Builder::new()
                .m(M)
                .ef_construction(EF_CONSTRUCTION)
                .prune_strategy(PruneStrategy::Heuristic)
                .seed(SEED)
                .build(Euclidean),
            buckets: HashMap::new(),
            slots: Vec::new(),
            entries: 0,
            indexed,
        }
    }
    fn insert(&mut self, vector: &[f32], slot: u32) {
        use std::hash::{Hash, Hasher};
        let mut hash = std::hash::DefaultHasher::new();
        for value in vector {
            value.to_bits().hash(&mut hash);
        }
        let bucket = self.buckets.entry(hash.finish()).or_default();
        let node = if let Some(node) = bucket
            .iter()
            .copied()
            .find(|&node| self.index.get_vector(node) == vector)
        {
            node
        } else {
            let node = self.index.insert(vector.to_vec());
            bucket.push(node);
            self.slots.push(Vec::new());
            node
        };
        self.slots[node].push(slot);
        self.entries += 1;
        self.indexed = self.indexed.max(slot as usize + 1);
    }
    fn heap_bytes(&self) -> usize {
        let nodes = self.index.len();
        let level = self.index.max_level().unwrap_or(0);
        // Coarse retained estimate: doubled Vec growth, full links at the
        // greatest observed level for every node. Excludes transient search buffers.
        nodes.saturating_mul(8 * self.index.dim().unwrap_or(0) + 96 + (level + 1) * (24 + 16 * M))
            + self.entries.saturating_mul(8)
            + self.buckets.capacity() * (size_of::<u64>() + size_of::<Vec<usize>>() + 1)
            + (EF_CONSTRUCTION * 96)
            + self.partition.len() * size_of::<NormalizedValue>()
            + self.partition.iter().map(value_heap_bytes).sum::<usize>()
    }
}

#[derive(Default)]
pub(crate) struct AnnIndex {
    // Hash collisions cannot combine tenants: actual typed partitions are checked.
    graphs: HashMap<(usize, u64), Vec<Graph>>,
}
impl AnnIndex {
    pub(crate) fn update(
        &mut self,
        store: &RecordStore,
        vectors: &RecordVectors,
        partitions: &HashMap<u64, Vec<u32>>,
        slots: Range<usize>,
    ) {
        for slot in slots {
            let record = &store.records()[slot];
            let partition = partition_fingerprint(record);
            let partition_slots = &partitions[&partition];
            if partition_slots.len() < MIN_PARTITION {
                continue;
            }
            for field in 0..store.schema().fields().len() {
                let Some(vectors) = vectors.field(field) else {
                    continue;
                };
                let Some(vector) = vectors.get(slot) else {
                    continue;
                };
                let graphs = self.graphs.entry((field, partition)).or_default();
                if let Some(graph) = graphs
                    .iter_mut()
                    .find(|graph| graph.partition.as_ref() == record.partition())
                {
                    if slot >= graph.indexed {
                        graph.insert(vector, u32::try_from(slot).unwrap_or(u32::MAX));
                    }
                } else {
                    let mut graph = Graph::new(record, store.len());
                    for &slot in partition_slots {
                        if store.is_active(slot as usize)
                            && store.records()[slot as usize].partition() == record.partition()
                            && let Some(vector) = vectors.get(slot as usize)
                        {
                            graph.insert(vector, slot);
                        }
                    }
                    graphs.push(graph);
                }
            }
        }
    }

    /// Upper bound for routing comparisons, including adaptive retries and
    /// cosine verification. Each level visits a graph node at most once.
    pub(crate) fn comparison_bound(
        &self,
        field: usize,
        partition: u64,
        retained: usize,
        options: AnnOptions,
    ) -> usize {
        let mut requested = options.max_candidates.min(32);
        let mut rounds = 1usize;
        while requested < options.max_candidates {
            requested = (requested * 2).min(options.max_candidates);
            rounds += 1;
        }
        self.graphs
            .get(&(field, partition))
            .map_or(retained, |graphs| {
                graphs
                    .iter()
                    .map(|graph| {
                        graph
                            .index
                            .len()
                            .saturating_mul(graph.index.max_level().unwrap_or(0) + 2)
                            .saturating_add(2)
                            .saturating_mul(rounds)
                    })
                    .max()
                    .unwrap_or(0)
                    .max(retained)
            })
    }

    pub(crate) fn candidates(
        &self,
        store: &RecordStore,
        (query, vector): (&PreparedRecord, &[f32]),
        field: usize,
        threshold: f32,
        options: AnnOptions,
        (exclude_slot, min_slot): (Option<usize>, usize),
    ) -> Option<(Vec<usize>, bool)> {
        let graph = self
            .graphs
            .get(&(field, partition_fingerprint(query)))?
            .iter()
            .find(|graph| graph.partition.as_ref() == query.partition())?;
        let nodes = graph.index.len();
        if nodes == 0 {
            return None;
        }
        let ceiling = options.max_candidates.min(nodes);
        let mut requested = ceiling.min(32);
        loop {
            let found = graph.index.search(vector, requested, options.ef_search);
            let mut slots = Vec::new();
            let mut saturated = false;
            for point in &found {
                if semantic_similarity(vector, graph.index.get_vector(point.id)) < threshold {
                    continue;
                }
                for &slot in &graph.slots[point.id] {
                    let slot = slot as usize;
                    let record = &store.records()[slot];
                    if slot < min_slot
                        || Some(slot) == exclude_slot
                        || !store.is_active(slot)
                        || (query.id().is_some() && query.id() == record.id())
                    {
                        continue;
                    }
                    if slots.len() == options.max_candidates {
                        saturated = true;
                        break;
                    }
                    slots.push(slot);
                }
                if saturated {
                    break;
                }
            }
            let expand = found.len() == requested
                && found.last().is_some_and(|point| {
                    semantic_similarity(vector, graph.index.get_vector(point.id)) >= threshold
                });
            if saturated || !expand || requested == ceiling {
                return Some((slots, saturated || (expand && ceiling < nodes)));
            }
            requested = (requested * 2).min(ceiling);
        }
    }
    pub(crate) fn heap_bytes(&self) -> usize {
        self.graphs
            .values()
            .flatten()
            .map(Graph::heap_bytes)
            .sum::<usize>()
            + self.graphs.capacity() * (size_of::<(usize, u64)>() + size_of::<Vec<Graph>>() + 1)
            + self
                .graphs
                .values()
                .map(|graphs| graphs.capacity() * size_of::<Graph>())
                .sum::<usize>()
    }
    pub(crate) fn nodes(&self) -> usize {
        self.graphs
            .values()
            .flatten()
            .map(|graph| graph.index.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fast_hnsw::distance::Distance;
    use std::hash::{Hash, Hasher};
    use univo_core::{FieldPath, Map, Schema, Text, Value, prepare};

    fn record(schema: &Schema, id: i64, tenant: &str) -> PreparedRecord {
        prepare(
            schema,
            &Value::Map(
                Map::new(vec![
                    ("id".into(), Value::Int(id)),
                    ("tenant".into(), Value::Str(tenant.into())),
                    ("name".into(), Value::Str("same".into())),
                ])
                .unwrap(),
            ),
        )
        .unwrap()
    }

    #[test]
    fn portable_distance_matches_a_naive_oracle_at_lane_boundaries() {
        for length in 0u8..40 {
            let a: Vec<_> = (0..length).map(|i| f32::from(i) * 0.07).collect();
            let b: Vec<_> = (0..length).map(|i| (f32::from(i) * 0.11).cos()).collect();
            let expected = a
                .iter()
                .zip(&b)
                .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!((f64::from(Euclidean.distance(&a, &b)) - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn comparison_budget_includes_ann_levels_and_adaptive_retries() {
        let schema = Schema::strings();
        let record = prepare(&schema, &Value::Str("same".into())).unwrap();
        let mut graph = Graph::new(&record, 0);
        for slot in 0u16..100 {
            graph.insert(&[f32::from(slot), 1.0], u32::from(slot));
        }
        let levels = graph.index.max_level().unwrap_or(0) + 2;
        let mut ann = AnnIndex::default();
        ann.graphs.insert((0, 1), vec![graph]);
        assert_eq!(
            ann.comparison_bound(0, 1, 100, AnnOptions::default()),
            (100 * levels + 2) * 6
        );
        assert_eq!(ann.comparison_bound(0, 2, 100, AnnOptions::default()), 100);
        assert_eq!(
            ann.comparison_bound(0, 1, 100_000, AnnOptions::default()),
            100_000
        );
    }

    #[test]
    fn vector_hash_collisions_do_not_share_graph_rows() {
        let schema = Schema::strings();
        let record = prepare(&schema, &Value::Str("same".into())).unwrap();
        let mut graph = Graph::new(&record, 0);
        graph.insert(&[1.0, 0.0], 0);
        // Force the next vector hash to propose node 0.
        let mut hash = std::hash::DefaultHasher::new();
        for value in [0.0f32, 1.0] {
            value.to_bits().hash(&mut hash);
        }
        graph.buckets.insert(hash.finish(), vec![0]);
        graph.insert(&[0.0, 1.0], 1);
        assert_eq!(graph.index.len(), 2);
        assert_eq!(graph.index.get_vector(1), [0.0, 1.0]);
        assert_eq!(graph.slots, [vec![0], vec![1]]);
    }

    #[test]
    fn colliding_partition_keys_are_verified_before_ann_traversal() {
        let schema = Schema::builder()
            .id_field(FieldPath::parse("id").unwrap())
            .partition_by(FieldPath::parse("tenant").unwrap())
            .text(FieldPath::parse("name").unwrap(), Text::default())
            .build()
            .unwrap();
        let a = record(&schema, 0, "a");
        let b = record(&schema, 1, "b");
        let query = record(&schema, -1, "a");
        let mut own = Graph::new(&a, 0);
        own.insert(&[1.0, 0.0], 0);
        let mut foreign = Graph::new(&b, 0);
        foreign.insert(&[1.0, 0.0], 1);
        let mut ann = AnnIndex::default();
        ann.graphs
            .insert((0, partition_fingerprint(&query)), vec![foreign, own]);
        let mut store = RecordStore::new(schema, 2);
        store.append(vec![a, b]).unwrap();
        assert_eq!(
            ann.candidates(
                &store,
                (&query, &[1.0, 0.0]),
                0,
                0.8,
                AnnOptions::default(),
                (None, 0)
            ),
            Some((vec![0], false))
        );
    }
}
