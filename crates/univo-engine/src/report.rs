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
                        let result = self.visit_pairs(
                            left,
                            options,
                            exhaustive,
                            dense,
                            &mut |right, score| {
                                found.keep(RankedPair {
                                    score,
                                    left,
                                    right_id: self.store().records()[right].id(),
                                    right,
                                });
                                Ok(())
                            },
                        )?;
                        found.add(&result);
                        Ok(())
                    },
                )?;
                Ok(found)
            },
            Found::merge,
        )?;
        // Only the pairs kept are explained, in parallel inside a pool.
        let mut report = found.report;
        let kept = found.kept.into_sorted_vec();
        let explain = |ranked: &RankedPair<'_>| {
            Ok(PairMatch {
                left: ranked.left,
                right: ranked.right,
                comparison: self.explain_pair(ranked.left, ranked.right, ranked.score)?,
            })
        };
        report.matches = if rayon::current_thread_index().is_some() {
            use rayon::prelude::*;
            kept.par_iter().map(explain).collect::<Result<_>>()?
        } else {
            kept.iter().map(explain).collect::<Result<_>>()?
        };
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

    /// Adds the counts of one query, whose matches went to [`Self::keep`].
    fn add(&mut self, result: &crate::SearchResult) {
        self.report.total_matches += result.total_matches;
        self.report.candidates += result.candidates;
        self.report.candidate_truncated |= result.truncated;
        self.report.approximate |= result.approximate;
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

/// A kept pair, explained only once the report is complete. The greatest
/// pair is the worst kept result, so replacement is O(log limit).
struct RankedPair<'a> {
    score: f32,
    left: usize,
    right_id: Option<&'a RecordId>,
    right: usize,
}
impl Ord for RankedPair<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then_with(|| self.left.cmp(&other.left))
            .then_with(|| self.right_id.cmp(&other.right_id))
            .then_with(|| self.right.cmp(&other.right))
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
