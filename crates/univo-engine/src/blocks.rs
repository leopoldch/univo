//! The dense lane of all-pairs reports, a block of queries at a time.
//!
//! Each all-pairs query scans every later vector of its partition. One query
//! at a time, a large partition streams all its vectors from memory for
//! every query, and memory bandwidth bounds the scan, whatever the threads
//! ([ADR 0019](../../../docs/adr/0019-blocked-dense-all-pairs.md)). A block
//! of consecutive queries of one partition reads each later vector once and
//! compares it with every query of the block, with the arithmetic of
//! [`univo_core::dot`]: the candidates are the ones the per-query scan
//! finds, bit for bit.

use std::collections::HashMap;

use univo_core::{dot, dots, partition_fingerprint};

use crate::SearchIndex;
use crate::search::lane_threshold;

/// Queries per block: their vectors, 1 KiB each at 256 dimensions, stay in
/// a core's cache while the partition streams past them.
pub(crate) const QUERY_BLOCK: usize = 64;

/// A query of the block with an embedding for the field being scanned.
struct Member<'a> {
    index: usize,
    slot: usize,
    vector: &'a [f32],
}

impl SearchIndex {
    /// `queries`, in increasing slot order, cut into blocks of at most
    /// [`QUERY_BLOCK`] queries of one partition.
    pub(crate) fn query_blocks(&self, queries: &[usize]) -> Vec<Vec<usize>> {
        let records = self.store().records();
        let mut partitions: HashMap<u64, Vec<usize>> = HashMap::new();
        for &slot in queries {
            partitions
                .entry(partition_fingerprint(&records[slot]))
                .or_default()
                .push(slot);
        }
        partitions
            .into_values()
            .flat_map(|slots| {
                slots
                    .chunks(QUERY_BLOCK)
                    .map(<[usize]>::to_vec)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// For each query of `block` (increasing slots of one partition) and
    /// each semantic field, the later active slots whose similarity reaches
    /// the lane threshold of `min_score`: what the dense lane finds for each
    /// query alone. `None` when the dense lane does not apply: no semantic
    /// field, or approximate retrieval.
    pub(crate) fn dense_block(
        &self,
        block: &[usize],
        min_score: f32,
    ) -> Option<Vec<Vec<Vec<u32>>>> {
        let vectors = self.vectors.as_ref()?;
        if self.ann_options.is_some() || block.is_empty() {
            return None;
        }
        let threshold = lane_threshold(min_score);
        let mut found = vec![vec![Vec::new(); self.semantic.len()]; block.len()];
        let partition = self.partition(&self.store().records()[block[0]]);
        for (position, &field) in self.semantic.iter().enumerate() {
            let Some(field_vectors) = vectors.field(field) else {
                continue;
            };
            let members: Vec<Member<'_>> = block
                .iter()
                .enumerate()
                .filter_map(|(index, &slot)| {
                    Some(Member {
                        index,
                        slot,
                        vector: field_vectors.get(slot)?,
                    })
                })
                .collect();
            let Some(first) = members.first() else {
                continue;
            };
            // Members before `ready` precede the current row: they scan it.
            let mut ready = 0;
            let start = partition.partition_point(|&slot| (slot as usize) <= first.slot);
            for &row_slot in &partition[start..] {
                let slot = row_slot as usize;
                if !self.store().is_active(slot) {
                    continue;
                }
                let Some(row) = field_vectors.get(slot) else {
                    continue;
                };
                while ready < members.len() && members[ready].slot < slot {
                    ready += 1;
                }
                let (eights, rest) = members[..ready].as_chunks::<8>();
                for group in eights {
                    let similarities = dots(group.each_ref().map(|member| member.vector), row);
                    for (member, similarity) in group.iter().zip(similarities) {
                        if similarity.clamp(0.0, 1.0) >= threshold {
                            found[member.index][position].push(row_slot);
                        }
                    }
                }
                for member in rest {
                    if dot(member.vector, row).clamp(0.0, 1.0) >= threshold {
                        found[member.index][position].push(row_slot);
                    }
                }
            }
        }
        Some(found)
    }
}
