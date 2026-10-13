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
    pub approximate: bool,
}

impl SearchIndex {
    /// Compares every stored query, keeping each unordered pair once and at
    /// most `options.limit` pairs. The oracle scores all admissible pairs.
    ///
    /// Queries run in parallel when called inside a Rayon pool, which then
    /// bounds the threads, and in order otherwise. Each worker keeps its
    /// best `limit` pairs; the order is total, so the result does not
    /// depend on the number of threads.
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
        self.check_scan(exhaustive, max_pairs, max_semantic_comparisons)?;
        let queries: Vec<usize> = self.store().active_slots().collect();
        let found = crate::fold_queries(
            self.query_blocks(&queries),
            || Found::new(options.limit),
            |mut found, block| {
                self.for_each_in_block(
                    &block,
                    options.min_score,
                    exhaustive,
                    &mut |left, dense| {
                        let result = self.search_pairs(left, options, exhaustive, dense)?;
                        found.add(self, left, result);
                        Ok(())
                    },
                )?;
                Ok(found)
            },
            Found::merge,
        )?;
        let mut report = found.report;
        report.matches = found
            .kept
            .into_sorted_vec()
            .into_iter()
            .map(|ranked| ranked.found)
            .collect();
        Ok(report)
    }
}

/// The counts of a set of queries and their best `limit` pairs.
struct Found<'a> {
    report: PairReport,
    kept: BinaryHeap<RankedPair<'a>>,
    limit: usize,
}

impl<'a> Found<'a> {
    fn new(limit: usize) -> Self {
        Self {
            report: PairReport::default(),
            kept: BinaryHeap::with_capacity(limit.min(1024).saturating_add(1)),
            limit,
        }
    }

    fn add(&mut self, index: &'a SearchIndex, left: usize, result: crate::SearchResult) {
        self.report.total_matches += result.total_matches;
        self.report.candidates += result.candidates;
        self.report.candidate_truncated |= result.truncated;
        self.report.approximate |= result.approximate;
        for found in result.matches {
            self.keep(RankedPair {
                right_id: index.store().records()[found.slot].id(),
                found: PairMatch {
                    left,
                    right: found.slot,
                    comparison: found.comparison,
                },
            });
        }
    }

    fn keep(&mut self, candidate: RankedPair<'a>) {
        if self.kept.len() < self.limit {
            self.kept.push(candidate);
        } else if self.kept.peek().is_some_and(|worst| candidate < *worst) {
            self.kept.pop();
            self.kept.push(candidate);
        }
    }

    fn merge(mut self, mut other: Self) -> Self {
        if other.kept.len() > self.kept.len() {
            std::mem::swap(&mut self.kept, &mut other.kept);
        }
        self.report.total_matches += other.report.total_matches;
        self.report.candidates += other.report.candidates;
        self.report.candidate_truncated |= other.report.candidate_truncated;
        self.report.approximate |= other.report.approximate;
        for candidate in other.kept {
            self.keep(candidate);
        }
        self
    }
}

impl SearchIndex {
    /// Refuses an all-pairs scan above its budget, before any scoring: the
    /// admissible pairs when exhaustive, the semantic comparisons otherwise.
    pub(crate) fn check_scan(
        &self,
        exhaustive: bool,
        max_pairs: usize,
        max_semantic_comparisons: usize,
    ) -> Result<()> {
        if exhaustive && self.pair_count() > max_pairs {
            return Err(univo_core::Error::pair_limit(self.pair_count(), max_pairs));
        }
        if !exhaustive && self.semantic_scan_count() > max_semantic_comparisons {
            return Err(univo_core::Error::semantic_comparison_limit(
                self.semantic_scan_count(),
                max_semantic_comparisons,
            ));
        }
        Ok(())
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
