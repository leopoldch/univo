//! Exhaustive nearest-neighbour search over the embeddings of one field.
//!
//! Each query scans its candidates once, with `O(k)` memory: no pairwise
//! matrix is ever built. It is the oracle against which any approximate
//! index is measured, and a supported mode within its cost, `O(n·d)` per
//! query.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use univo_core::semantic_similarity;

use crate::vectors::FieldVectors;

/// A record close to the query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Neighbor {
    pub slot: usize,
    /// Semantic similarity, as in pairwise scoring: the cosine clamped to
    /// `[0, 1]`.
    pub similarity: f32,
}

/// A kept candidate; the greatest entry is the worst one.
struct Entry<K> {
    similarity: f32,
    key: K,
    slot: usize,
}

impl<K: Ord> Ord for Entry<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .similarity
            .total_cmp(&self.similarity)
            .then_with(|| self.key.cmp(&other.key))
            .then_with(|| self.slot.cmp(&other.slot))
    }
}

impl<K: Ord> PartialOrd for Entry<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: Ord> PartialEq for Entry<K> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl<K: Ord> Eq for Entry<K> {}

/// The `k` candidates whose embedding is closest to `query`, most similar
/// first: the first `k` results of [`within`] at threshold 0. Ties are
/// broken by `key`, then by slot, smallest first, so the result never
/// depends on the order of `candidates`. Candidates without an embedding
/// are skipped.
pub fn top_k<K: Ord>(
    vectors: &FieldVectors,
    query: &[f32],
    candidates: impl IntoIterator<Item = usize>,
    k: usize,
    key: impl Fn(usize) -> K,
) -> Vec<Neighbor> {
    if k == 0 || query.len() != vectors.dimension() {
        return Vec::new();
    }
    let mut kept: BinaryHeap<Entry<K>> = BinaryHeap::with_capacity(k + 1);
    for slot in candidates {
        let Some(row) = vectors.get(slot) else {
            continue;
        };
        let similarity = semantic_similarity(query, row);
        if kept.len() == k {
            let worst = kept
                .peek()
                .map_or(f32::NEG_INFINITY, |entry| entry.similarity);
            // Cheap rejection before building the tie-break key.
            if similarity < worst {
                continue;
            }
            let entry = Entry {
                similarity,
                key: key(slot),
                slot,
            };
            if kept.peek().is_some_and(|worst| entry < *worst) {
                kept.pop();
                kept.push(entry);
            }
        } else {
            kept.push(Entry {
                similarity,
                key: key(slot),
                slot,
            });
        }
    }
    kept.into_sorted_vec()
        .into_iter()
        .map(|entry| Neighbor {
            slot: entry.slot,
            similarity: entry.similarity,
        })
        .collect()
}

/// Every candidate whose similarity to `query` is at least `threshold`,
/// most similar first, ties broken by `key` then slot: the complete answer,
/// at the cost of a result as large as the matches.
pub fn within<K: Ord>(
    vectors: &FieldVectors,
    query: &[f32],
    candidates: impl IntoIterator<Item = usize>,
    threshold: f32,
    key: impl Fn(usize) -> K,
) -> Vec<Neighbor> {
    if query.len() != vectors.dimension() {
        return Vec::new();
    }
    let mut found: Vec<(Neighbor, K)> = candidates
        .into_iter()
        .filter_map(|slot| {
            let similarity = semantic_similarity(query, vectors.get(slot)?);
            (similarity >= threshold).then(|| (Neighbor { slot, similarity }, key(slot)))
        })
        .collect();
    found.sort_by(|(a, a_key), (b, b_key)| {
        b.similarity
            .total_cmp(&a.similarity)
            .then_with(|| a_key.cmp(b_key))
            .then_with(|| a.slot.cmp(&b.slot))
    });
    found.into_iter().map(|(neighbor, _)| neighbor).collect()
}

/// The slots of [`within`], unsorted, appended to `found`; returns their
/// number. Candidate lanes sort and deduplicate all their slots anyway.
pub(crate) fn extend_within(
    vectors: &FieldVectors,
    query: &[f32],
    candidates: impl IntoIterator<Item = usize>,
    threshold: f32,
    found: &mut Vec<usize>,
) -> usize {
    if query.len() != vectors.dimension() {
        return 0;
    }
    let before = found.len();
    found.extend(candidates.into_iter().filter(|&slot| {
        vectors
            .get(slot)
            .is_some_and(|row| semantic_similarity(query, row) >= threshold)
    }));
    found.len() - before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extend_within_appends_the_slots_within_finds() {
        // Slot 3 has no embedding; rows 0 to 4 are unit vectors at angles.
        let angles = [0.0f32, 0.3, 0.6, 0.9, 1.2];
        let vectors = FieldVectors {
            dimension: 2,
            data: angles.iter().flat_map(|a| [a.cos(), a.sin()]).collect(),
            rows: vec![Some(4), Some(0), Some(2), None, Some(1), Some(3)],
        };
        let query = [1.0, 0.0];
        for threshold in [0.0, 0.5, 0.8, 0.96, 1.0] {
            let mut expected: Vec<usize> = within(&vectors, &query, 0..6, threshold, |slot| slot)
                .iter()
                .map(|neighbor| neighbor.slot)
                .collect();
            let mut found = vec![42];
            let added = extend_within(&vectors, &query, 0..6, threshold, &mut found);
            assert_eq!(found.remove(0), 42);
            assert_eq!(added, found.len());
            expected.sort_unstable();
            assert_eq!(found, expected, "{threshold}");
        }
        assert_eq!(
            extend_within(&vectors, &[1.0], 0..6, 0.0, &mut Vec::new()),
            0
        );
    }
}
