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

use univo_core::{Result, dot, dots_reaching, partition_fingerprint, rest_norm};

use crate::SearchIndex;
use crate::search::lane_threshold;

/// Queries per block: their vectors, 1 KiB each at 256 dimensions, stay in
/// a core's cache while the partition streams past them.
pub(crate) const QUERY_BLOCK: usize = 64;

/// Candidates a block may hold, over all its queries and semantic fields:
/// 4 MiB of slots per worker, before the spare capacity of their vectors.
/// A block that would hold more is split in two and recomputed, down to one
/// query, which holds what the per-query scan held and may exceed it.
pub(crate) const DENSE_BLOCK_BUDGET: usize = 1 << 20;

/// One query's dense candidates: for each semantic field, the slots found.
pub(crate) type DenseCandidates = [Vec<u32>];

/// The dense candidates of a block.
pub(crate) enum Dense {
    /// The dense lane does not apply: no semantic field, approximate
    /// retrieval or an exhaustive search.
    NotApplicable,
    /// For each query, for each semantic field, the slots it found.
    Found(Vec<Vec<Vec<u32>>>),
    /// The block found more candidates than its budget.
    OverBudget,
}

/// A query of the block with an embedding for the field being scanned.
struct Member<'a> {
    index: usize,
    slot: usize,
    vector: &'a [f32],
    /// Norm of the dimensions after the prefix [`dots_reaching`] reads first.
    rest: f32,
}

/// Chunks of eight dimensions read before deciding whether a row can reach
/// the threshold for any query of a group: half of them. On ROR names with
/// the default model, about four rows in five are then skipped at 0.8.
fn prefix_chunks(dimension: usize) -> usize {
    dimension / 16
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

    /// Hands each query of `block` to `visit` with its dense candidates,
    /// or `None` for the dense lane's own scan, in slot order. Blocks over
    /// [`DENSE_BLOCK_BUDGET`] are split in two, so a block holds at most the
    /// budget or one query's candidates.
    pub(crate) fn for_each_in_block(
        &self,
        block: &[usize],
        min_score: f32,
        exhaustive: bool,
        visit: &mut dyn FnMut(usize, Option<&DenseCandidates>) -> Result<()>,
    ) -> Result<()> {
        let budget = if block.len() > 1 {
            self.dense_block_budget
        } else {
            usize::MAX
        };
        match self.dense_block(block, min_score, exhaustive, budget) {
            Dense::NotApplicable => block.iter().try_for_each(|&slot| visit(slot, None)),
            Dense::Found(found) => block
                .iter()
                .zip(&found)
                .try_for_each(|(&slot, dense)| visit(slot, Some(dense))),
            Dense::OverBudget => {
                let (first, second) = block.split_at(block.len() / 2);
                self.for_each_in_block(first, min_score, exhaustive, visit)?;
                self.for_each_in_block(second, min_score, exhaustive, visit)
            }
        }
    }

    /// For each query of `block` (increasing slots of one partition) and
    /// each semantic field, the later active slots whose similarity reaches
    /// the lane threshold of `min_score`: what the dense lane finds for each
    /// query alone, unless they exceed `budget` candidates in all.
    pub(crate) fn dense_block(
        &self,
        block: &[usize],
        min_score: f32,
        exhaustive: bool,
        budget: usize,
    ) -> Dense {
        let Some(vectors) = self.vectors.as_ref() else {
            return Dense::NotApplicable;
        };
        if exhaustive || self.ann_options.is_some() || block.is_empty() {
            return Dense::NotApplicable;
        }
        let threshold = lane_threshold(min_score);
        let mut held = 0usize;
        let mut found = vec![vec![Vec::new(); self.semantic.len()]; block.len()];
        let partition = self.partition(&self.store().records()[block[0]]);
        for (position, &field) in self.semantic.iter().enumerate() {
            let Some(field_vectors) = vectors.field(field) else {
                continue;
            };
            let prefix = prefix_chunks(field_vectors.dimension());
            let members: Vec<Member<'_>> = block
                .iter()
                .enumerate()
                .filter_map(|(index, &slot)| {
                    let vector = field_vectors.get(slot)?;
                    Some(Member {
                        index,
                        slot,
                        vector,
                        rest: rest_norm(vector, prefix),
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
                if held > budget {
                    return Dense::OverBudget;
                }
                let (eights, rest) = members[..ready].as_chunks::<8>();
                let row_rest = if eights.is_empty() {
                    0.0
                } else {
                    rest_norm(row, prefix)
                };
                for group in eights {
                    // Exact: a group is skipped only when no query can reach
                    // the threshold, and computed otherwise as `dots` does.
                    let Some(similarities) = dots_reaching(
                        group.each_ref().map(|member| member.vector),
                        group.each_ref().map(|member| member.rest),
                        row,
                        row_rest,
                        prefix,
                        threshold,
                    ) else {
                        continue;
                    };
                    for (member, similarity) in group.iter().zip(similarities) {
                        if similarity.clamp(0.0, 1.0) >= threshold {
                            found[member.index][position].push(row_slot);
                            held += 1;
                        }
                    }
                }
                for member in rest {
                    if dot(member.vector, row).clamp(0.0, 1.0) >= threshold {
                        found[member.index][position].push(row_slot);
                        held += 1;
                    }
                }
            }
        }
        if held > budget {
            return Dense::OverBudget;
        }
        Dense::Found(found)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use univo_core::{FieldPath, Map, Schema, Text, TextMode, Value, prepare};
    use univo_embed::{Embedder, Embeddings, ModelDescriptor, ModelSource};

    use super::{Dense, QUERY_BLOCK};
    use crate::SearchIndex;

    /// Every text gets the same unit vector: every pair matches.
    struct Constant(ModelDescriptor);

    impl Embedder for Constant {
        fn descriptor(&self) -> &ModelDescriptor {
            &self.0
        }

        fn encode_batch(&self, texts: &[&str]) -> univo_core::Result<Embeddings> {
            Embeddings::new("constant", vec![0.5; texts.len() * 4], 4)
        }
    }

    const FIELDS: usize = 8;

    fn index(records: usize) -> SearchIndex {
        let mut builder = Schema::builder();
        for field in 0..FIELDS {
            builder = builder.text(
                FieldPath::parse(&format!("f{field}")).unwrap(),
                Text {
                    mode: TextMode::Semantic,
                    ..Text::default()
                },
            );
        }
        let schema = builder.build().unwrap();
        let prepared = (0..records)
            .map(|row| {
                let entries = (0..FIELDS)
                    .map(|field| (format!("f{field}"), Value::Str(format!("text {row}"))))
                    .collect();
                prepare(&schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
            })
            .collect();
        let model: Arc<dyn Embedder> = Arc::new(Constant(ModelDescriptor {
            source: ModelSource::Local {
                path: "constant".into(),
                files: Vec::new(),
            },
            dimension: 4,
            backend: "constant",
        }));
        let mut index = SearchIndex::new(schema, records, Some(model)).unwrap();
        index.add(prepared).unwrap();
        index
    }

    #[test]
    fn a_block_over_its_budget_is_refused() {
        let index = index(500);
        let block: Vec<usize> = (0..QUERY_BLOCK).collect();
        // Every query matches every later record on every field.
        let all: usize = block.iter().map(|&slot| (499 - slot) * FIELDS).sum();
        let Dense::Found(found) = index.dense_block(&block, 0.8, false, all) else {
            panic!("the exact budget suffices");
        };
        let held: usize = found.iter().flatten().map(Vec::len).sum();
        assert_eq!(held, all);
        for budget in [0, 100, all - 1] {
            assert!(matches!(
                index.dense_block(&block, 0.8, false, budget),
                Dense::OverBudget
            ));
        }
    }

    #[test]
    fn split_blocks_visit_every_query_once_in_order() {
        let index = index(300).with_dense_block_budget(10);
        let block: Vec<usize> = (0..QUERY_BLOCK).collect();
        let mut visited = Vec::new();
        index
            .for_each_in_block(&block, 0.8, false, &mut |slot, dense| {
                let dense = dense.expect("dense candidates");
                // Down to one query, which holds all its candidates.
                assert_eq!(
                    dense.iter().map(Vec::len).sum::<usize>(),
                    (299 - slot) * FIELDS
                );
                visited.push(slot);
                Ok(())
            })
            .unwrap();
        assert_eq!(visited, block);
    }
}
