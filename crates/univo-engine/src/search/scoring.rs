//! Scoring candidates against a query: admissibility, scores without
//! explanations, bounded ranking, and explanations of the kept matches.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use univo_core::{
    Comparison, Error, PreparedRecord, RecordId, Result, Schema, Signal, compare_with_vectors,
    has_value, is_exact_match, score_with_vectors,
};

use super::{Query, SearchIndex, SearchMatch, SearchOptions, SearchResult, check_options};

impl SearchIndex {
    /// What `slot` is against the query, scored without explanations.
    /// Records equal to the query without a score, such as unknown semantic
    /// texts, score as exact. `vectors` is a buffer reused across
    /// candidates.
    fn qualifying<'a>(
        &'a self,
        query: &Query<'_>,
        slot: usize,
        vectors: &mut Vec<Option<&'a [f32]>>,
    ) -> Result<Candidate> {
        if !self.store().is_active(slot) || slot < query.min_slot || Some(slot) == query.slot {
            return Ok(Candidate::Excluded);
        }
        let record = &self.store().records()[slot];
        if query.record.id().is_some() && query.record.id() == record.id() {
            return Ok(Candidate::Excluded);
        }
        match &self.vectors {
            Some(stored) => stored.record_into(slot, vectors),
            None => vectors.clear(),
        }
        let Some(scored) =
            score_with_vectors(self.schema(), query.record, &query.vectors, record, vectors)?
        else {
            return Ok(Candidate::NoMatch);
        };
        let score = match scored.score {
            Some(score) => score,
            None if is_exact_match(self.schema(), query.record, record)? => 1.0,
            None => return Ok(Candidate::NoMatch),
        };
        Ok(if scored.vetoed {
            Candidate::NoMatch
        } else {
            Candidate::Scored(score)
        })
    }

    /// The explanation of a match [`Self::qualifying`] scored, computed
    /// again: only the matches kept are explained.
    fn explain(&self, query: &Query<'_>, slot: usize, score: f32) -> Result<Comparison> {
        let record = &self.store().records()[slot];
        let vectors = self
            .vectors
            .as_ref()
            .map_or_else(Vec::new, |vectors| vectors.record(slot));
        // Same inputs as the score: same partition, so never `None`.
        let mut comparison = compare_with_vectors(
            self.schema(),
            query.record,
            &query.vectors,
            record,
            &vectors,
        )?
        .ok_or_else(Error::schema_mismatch)?;
        if comparison.score.is_none() && is_exact_match(self.schema(), query.record, record)? {
            exact_evidence(self.schema(), query.record, &mut comparison);
        }
        debug_assert_eq!(comparison.score.map(f32::to_bits), Some(score.to_bits()));
        Ok(comparison)
    }

    /// The explanation of the match of stored records `left` and `right`.
    pub(crate) fn explain_pair(&self, left: usize, right: usize, score: f32) -> Result<Comparison> {
        self.explain(&self.stored_query(left)?, right, score)
    }

    /// Scores `slots`, each given once, against the query, and hands each
    /// match to `visit` without explaining it.
    fn score_each(
        &self,
        query: &Query<'_>,
        slots: impl Iterator<Item = usize>,
        min_score: f32,
        result: &mut SearchResult,
        visit: &mut dyn FnMut(usize, f32) -> Result<()>,
    ) -> Result<()> {
        let mut vectors = Vec::new();
        for slot in slots {
            match self.qualifying(query, slot, &mut vectors)? {
                Candidate::Excluded => {}
                Candidate::NoMatch => result.candidates += 1,
                Candidate::Scored(score) => {
                    result.candidates += 1;
                    if score >= min_score {
                        result.total_matches += 1;
                        visit(slot, score)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Scores `slots`, each given once, against the query. Every match is
    /// counted, and only the best `limit` are kept, in a bounded heap, then
    /// explained: a large group of duplicates costs `O(limit)` memory and
    /// explanations, not one per match.
    pub(super) fn score(
        &self,
        query: &Query<'_>,
        slots: impl Iterator<Item = usize>,
        options: SearchOptions,
        result: &mut SearchResult,
    ) -> Result<()> {
        let records = self.store().records();
        let mut kept: BinaryHeap<Ranked<'_>> =
            BinaryHeap::with_capacity(options.limit.min(1024).saturating_add(1));
        self.score_each(
            query,
            slots,
            options.min_score,
            result,
            &mut |slot, score| {
                let candidate = Ranked {
                    score,
                    id: records[slot].id(),
                    slot,
                };
                if kept.len() < options.limit {
                    kept.push(candidate);
                } else if kept.peek().is_some_and(|worst| candidate < *worst) {
                    kept.pop();
                    kept.push(candidate);
                }
                Ok(())
            },
        )?;
        result.matches = kept
            .into_sorted_vec()
            .into_iter()
            .map(|ranked| {
                Ok(SearchMatch {
                    slot: ranked.slot,
                    comparison: self.explain(query, ranked.slot, ranked.score)?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(())
    }

    /// Hands each match of the stored record at `slot` with a higher slot to
    /// `visit`, without keeping explanations: what all-pairs reports need
    /// when their own budget, not a limit, bounds memory. The returned
    /// result holds the counts and flags, without matches.
    pub(crate) fn visit_pairs(
        &self,
        slot: usize,
        options: SearchOptions,
        exhaustive: bool,
        dense: Option<&[Vec<u32>]>,
        visit: &mut dyn FnMut(usize, f32) -> Result<()>,
    ) -> Result<SearchResult> {
        check_options(options)?;
        let mut query = self.stored_query(slot)?;
        query.min_slot = slot + 1;
        query.dense = dense;
        let mut result = SearchResult::default();
        if exhaustive {
            // Partitions list slots in increasing order; the earlier ones
            // are excluded anyway.
            let partition = self.partition(query.record);
            let later = partition
                [partition.partition_point(|&slot| (slot as usize) < query.min_slot)..]
                .iter()
                .map(|&slot| slot as usize);
            self.score_each(&query, later, options.min_score, &mut result, visit)?;
        } else {
            let candidates = self.candidates(&query, options, &mut result)?;
            self.score_each(
                &query,
                candidates.into_iter(),
                options.min_score,
                &mut result,
                visit,
            )?;
        }
        Ok(result)
    }

    /// The score of stored records `left` and `right` as search scores
    /// them, or `None` when they cannot match: no score, or a veto.
    pub(crate) fn pair_score(&self, left: usize, right: usize) -> Result<Option<f32>> {
        let query = self.stored_query(left)?;
        Ok(match self.qualifying(&query, right, &mut Vec::new())? {
            Candidate::Scored(score) => Some(score),
            Candidate::Excluded | Candidate::NoMatch => None,
        })
    }
}

/// A candidate seen from a query.
enum Candidate {
    /// Not admissible: inactive, the query itself or its typed ID.
    Excluded,
    /// Compared, but it cannot match: no score, or a veto.
    NoMatch,
    Scored(f32),
}

/// A kept match. The greatest is the worst: lowest score, then highest
/// typed ID (strings before integers), then highest slot.
struct Ranked<'a> {
    score: f32,
    id: Option<&'a RecordId>,
    slot: usize,
}

impl Ord for Ranked<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then_with(|| self.id.cmp(&other.id))
            .then_with(|| self.slot.cmp(&other.slot))
    }
}

impl PartialOrd for Ranked<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Ranked<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Ranked<'_> {}

/// Whole-record equality is a proof of normalized equality even if the
/// encoder knows none of the text. Missing fields remain missing evidence.
fn exact_evidence(schema: &Schema, record: &PreparedRecord, comparison: &mut Comparison) {
    let mut total = 0.0f64;
    let mut backed = 0.0f64;
    for ((field, value), evidence) in schema
        .fields()
        .iter()
        .zip(record.fields())
        .zip(comparison.fields.iter_mut())
    {
        let weight = f64::from(field.kind.weight());
        total += weight;
        if has_value(field, value) {
            evidence.similarity = Some(1.0);
            evidence.signal = Signal::Exact;
            evidence.evidence = 1.0;
            backed += weight;
        }
    }
    comparison.score = Some(1.0);
    #[allow(clippy::cast_possible_truncation)]
    {
        comparison.coverage = (backed / total) as f32;
    }
}
