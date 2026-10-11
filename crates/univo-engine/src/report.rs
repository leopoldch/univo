//! Bounded pair reports over a temporary index. Matching stays in the engine.
use crate::{SearchIndex, SearchOptions};
use univo_core::{Comparison, Result};

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
    ) -> Result<PairReport> {
        // Validate even an empty dataset.
        if !(options.min_score.is_finite() && options.min_score > 0.0 && options.min_score <= 1.0) {
            return Err(univo_core::Error::invalid_threshold(options.min_score));
        }
        if exhaustive && self.pair_count() > max_pairs {
            return Err(univo_core::Error::pair_limit(self.pair_count(), max_pairs));
        }
        let mut report = PairReport::default();
        for left in 0..self.len() {
            let result = self.search_pairs(left, options, exhaustive)?;
            report.total_matches += result.total_matches;
            report.candidates += result.candidates;
            report.candidate_truncated |= result.truncated;
            for found in result.matches {
                report.matches.push(PairMatch {
                    left,
                    right: found.slot,
                    comparison: found.comparison,
                });
            }
            // At most twice the output limit, never an n-by-n matrix.
            report.matches.sort_by(|a, b| {
                b.comparison
                    .score
                    .unwrap_or(0.0)
                    .total_cmp(&a.comparison.score.unwrap_or(0.0))
                    .then_with(|| a.left.cmp(&b.left))
                    .then_with(|| {
                        self.store().records()[a.right]
                            .id()
                            .cmp(&self.store().records()[b.right].id())
                    })
                    .then_with(|| a.right.cmp(&b.right))
            });
            report.matches.truncate(options.limit);
        }
        Ok(report)
    }
}
