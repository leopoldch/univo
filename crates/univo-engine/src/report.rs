//! Bounded pair reports over a temporary index. Matching stays in the engine.
use crate::{SearchIndex, SearchOptions};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use univo_core::{Comparison, RecordId, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct PairMatch {
    pub left: usize,
    pub right: usize,
    pub comparison: Comparison,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PairReport {
    pub matches: Vec<PairMatch>,
    pub total_matches: usize,
    pub candidates: usize,
    pub candidate_truncated: bool,
}

impl SearchIndex {
    /// Compares every stored query, keeping each unordered pair once and at
    /// most `options.limit` pairs. The oracle scores all admissible pairs.
    ///
    /// # Errors
    /// Returns search option, schema and model errors from the search engine.
    pub fn find(
        &self,
        options: SearchOptions,
        exhaustive: bool,
        max_pairs: usize,
        max_semantic_comparisons: usize,
    ) -> Result<PairReport> {
        // Validate even an empty dataset.
        if !(options.min_score.is_finite() && options.min_score > 0.0 && options.min_score <= 1.0) {
            return Err(univo_core::Error::invalid_threshold(options.min_score));
        }
        if exhaustive && self.pair_count() > max_pairs {
            return Err(univo_core::Error::pair_limit(self.pair_count(), max_pairs));
        }
        if !exhaustive && self.semantic_scan_count() > max_semantic_comparisons {
            return Err(univo_core::Error::semantic_comparison_limit(
                self.semantic_scan_count(),
                max_semantic_comparisons,
            ));
        }
        let mut report = PairReport::default();
        let mut kept = BinaryHeap::with_capacity(options.limit.min(1024).saturating_add(1));
        for left in self.store().active_slots() {
            let result = self.search_pairs(left, options, exhaustive)?;
            report.total_matches += result.total_matches;
            report.candidates += result.candidates;
            report.candidate_truncated |= result.truncated;
            for found in result.matches {
                let candidate = RankedPair {
                    right_id: self.store().records()[found.slot].id(),
                    found: PairMatch {
                        left,
                        right: found.slot,
                        comparison: found.comparison,
                    },
                };
                if kept.len() < options.limit {
                    kept.push(candidate);
                } else if kept.peek().is_some_and(|worst| candidate < *worst) {
                    kept.pop();
                    kept.push(candidate);
                }
            }
        }
        report.matches = kept
            .into_sorted_vec()
            .into_iter()
            .map(|ranked| ranked.found)
            .collect();
        Ok(report)
    }
}

/// The greatest pair is the worst kept result, so replacement is O(log limit).
struct RankedPair<'a> {
    right_id: Option<&'a RecordId>,
    found: PairMatch,
}
impl Ord for RankedPair<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .found
            .comparison
            .score
            .unwrap_or(0.0)
            .total_cmp(&self.found.comparison.score.unwrap_or(0.0))
            .then_with(|| self.found.left.cmp(&other.found.left))
            .then_with(|| self.right_id.cmp(&other.right_id))
            .then_with(|| self.found.right.cmp(&other.found.right))
    }
}
impl PartialOrd for RankedPair<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for RankedPair<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for RankedPair<'_> {}
