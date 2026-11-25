//! All-pairs reports decided by weights learned from the records
//! themselves ([`univo_core::Weights`]), without labels.
//!
//! A pair is a candidate when a lane of the search proposes it at
//! [`PARTIAL_SIMILARITY`]: a field at least partially alike, or an equal
//! exact value, whatever the mean of its similarities. The report is made in
//! two passes:
//!
//! 1. **Learn.** Chance agreement is counted over pairs of one partition
//!    taken at random, every pair when the index has at most a million.
//!    Agreement inside an entity comes from the candidates of at most
//!    [`FIT_QUERIES`] records spread over the index.
//! 2. **Decide.** Every candidate gets the bits of its fields, an equal
//!    value weighing by how common it is among the active records, and a
//!    probability. The pairs reaching the minimum probability are counted
//!    and the heaviest are kept, so memory follows the limit, not the
//!    candidates.
//!
//! An equal exact value shared by many records proposes many pairs. In the
//! second pass a record's exact values propose candidates only when, all
//! equal, they could reach the minimum probability by themselves: otherwise
//! a pair needs a field at least partially alike, and the lane of that field
//! proposes it. No pair that reaches the minimum is lost to this.
//!
//! Vetoes and partitions apply as in any search: a vetoed pair is not a
//! candidate.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use rayon::prelude::*;
use univo_core::{
    Comparison, Error, FieldWeights, Fit, LEVELS, Level, PARTIAL_SIMILARITY, Result, Weights,
};

use crate::SearchIndex;
use crate::search::Candidates;

/// Pairs taken at random to count chance agreement.
const CHANCE_PAIRS: usize = 1_000_000;

/// Batches the random pairs are drawn in, each with its own seed, so that
/// the counts do not depend on the threads.
const CHANCE_BATCHES: usize = 64;

/// Records whose candidates the weights are learned from, at most.
pub const FIT_QUERIES: usize = 1024;

/// Index entries one lane reads for one of these records.
const FIT_ENTRIES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeightedOptions {
    /// Lowest probability returned, in `(0, 1]`.
    pub min_probability: f64,
    pub limit: usize,
    pub max_entries: usize,
}

/// A pair, why it matches and what each field weighs.
#[derive(Debug, Clone, PartialEq)]
pub struct WeightedMatch {
    pub left: usize,
    pub right: usize,
    pub comparison: Comparison,
    /// Bits each field adds, in schema order; `None` without evidence.
    pub bits: Box<[Option<f64>]>,
    /// Sum of the bits, before the prior.
    pub weight: f64,
    pub probability: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeightedReport {
    /// Heaviest first, then by slots.
    pub matches: Vec<WeightedMatch>,
    /// Pairs reaching the minimum probability.
    pub total_matches: usize,
    /// Pairs the candidate search scored.
    pub candidates: usize,
    pub candidate_truncated: bool,
    pub approximate: bool,
    pub weights: Weights,
    pub fit: Fit,
}

/// A kept pair. The greatest is the worst kept: the lightest, then the
/// latest slots.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ranked {
    weight: f64,
    left: usize,
    right: usize,
    score: f32,
}

impl Eq for Ranked {}

impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .weight
            .total_cmp(&self.weight)
            .then_with(|| (self.left, self.right).cmp(&(other.left, other.right)))
    }
}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The counts of a set of queries and their heaviest `limit` pairs.
struct Kept {
    heap: BinaryHeap<Ranked>,
    limit: usize,
    total: usize,
    scored: usize,
    truncated: bool,
    approximate: bool,
}

impl Kept {
    fn new(limit: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(limit.min(1024).saturating_add(1)),
            limit,
            total: 0,
            scored: 0,
            truncated: false,
            approximate: false,
        }
    }

    fn keep(&mut self, pair: Ranked) {
        self.total += 1;
        if self.heap.len() < self.limit {
            self.heap.push(pair);
        } else if self.heap.peek().is_some_and(|worst| pair < *worst) {
            self.heap.pop();
            self.heap.push(pair);
        }
    }

    fn merge(mut self, mut other: Self) -> Self {
        if other.heap.len() > self.heap.len() {
            std::mem::swap(&mut self.heap, &mut other.heap);
        }
        let total = self.total + other.total;
        for pair in other.heap {
            self.keep(pair);
        }
        self.total = total;
        self.scored += other.scored;
        self.truncated |= other.truncated;
        self.approximate |= other.approximate;
        self
    }
}

/// What deciding needs beyond the index: the weights and how common each
/// value of each field is among the active records.
struct Scale {
    weights: Weights,
    counts: Vec<HashMap<u64, u32>>,
    valued: Vec<u32>,
}

impl Scale {
    /// The share of the other records holding the value of `field` in the
    /// record at `slot`; `None` without a value.
    fn share(&self, index: &SearchIndex, field: usize, slot: usize) -> Option<f64> {
        let key = index.store().records()[slot].fields()[field].value_key()?;
        let (count, valued) = (self.counts[field].get(&key)?, self.valued[field]);
        let others = f64::from(valued.saturating_sub(1).max(1));
        Some((f64::from(count.saturating_sub(1)) / others).max(1.0 / f64::from(valued.max(1))))
    }

    /// The bits each field of a compared pair adds.
    fn bits(
        &self,
        index: &SearchIndex,
        left: usize,
        right: usize,
        comparison: &Comparison,
    ) -> Box<[Option<f64>]> {
        let records = index.store().records();
        comparison
            .fields
            .iter()
            .zip(&self.weights.fields)
            .enumerate()
            .map(|(field, (compared, learned))| {
                let level = Level::of(compared.similarity?);
                // The frequency of a value only speaks for values that are
                // the same, not for texts a model finds alike.
                let same = level == Level::Equal
                    && records[left].fields()[field].value_key()
                        == records[right].fields()[field].value_key();
                let share = same.then(|| self.share(index, field, left)).flatten();
                Some(learned.weight(level, share))
            })
            .collect()
    }

    /// The most bits the exact values of the record at `slot` can add, all
    /// of them equal in the other record.
    fn exact_at_best(&self, index: &SearchIndex, slot: usize) -> f64 {
        index
            .value_fields()
            .filter_map(|field| {
                let share = self.share(index, field, slot)?;
                Some(
                    self.weights.fields[field]
                        .weight(Level::Equal, Some(share))
                        .max(0.0),
                )
            })
            .sum()
    }
}

/// The level of each field of a comparison, as [`Weights::fit`] reads them.
fn codes(comparison: &Comparison) -> impl Iterator<Item = u8> + '_ {
    comparison
        .fields
        .iter()
        .map(|field| Level::code(field.similarity.map(Level::of)))
}

impl SearchIndex {
    /// Every pair of stored records that is one entity with a probability
    /// of at least `options.min_probability`, by weights learned from the
    /// index.
    ///
    /// Queries run in parallel when called inside a Rayon pool, and the
    /// result does not depend on the number of threads.
    ///
    /// # Errors
    ///
    /// Returns `parameter.threshold` unless `0 < min_probability ≤ 1`, the
    /// scan budget errors of `find`, and search errors.
    pub fn find_weighted(
        &self,
        options: WeightedOptions,
        exhaustive: bool,
        max_exhaustive_pairs: usize,
        max_semantic_comparisons: usize,
    ) -> Result<WeightedReport> {
        if !(options.min_probability > 0.0 && options.min_probability <= 1.0) {
            #[allow(clippy::cast_possible_truncation)]
            return Err(Error::invalid_threshold(options.min_probability as f32));
        }
        self.check_scan(exhaustive, max_exhaustive_pairs, max_semantic_comparisons)?;
        let queries: Vec<usize> = self.store().active_slots().collect();
        let (weights, fit) = self.fit_weights(&queries, exhaustive)?;
        let scale = self.scale(weights);
        // Bits a pair needs: the odds of the minimum, less the prior.
        let certain = options.min_probability.min(1.0 - f64::EPSILON);
        let needed = (certain / (1.0 - certain)).log2() - scale.weights.prior;

        let sketches = self.dense_sketches(queries.len(), exhaustive);
        let kept = crate::fold_queries(
            self.query_blocks(&queries),
            || Kept::new(options.limit),
            |mut kept, block| {
                self.for_each_in_block(
                    &block,
                    PARTIAL_SIMILARITY,
                    exhaustive,
                    &sketches,
                    &mut |left, dense| {
                        let wanted = Candidates {
                            lane: PARTIAL_SIMILARITY,
                            max_entries: options.max_entries,
                            exhaustive,
                            later: true,
                            values: scale.exact_at_best(self, left) >= needed,
                        };
                        let pairs = &mut kept;
                        let result =
                            self.visit_candidates(left, wanted, dense, &mut |right, score| {
                                let comparison = self.explain_pair(left, right, score)?;
                                let weight: f64 = scale
                                    .bits(self, left, right, &comparison)
                                    .iter()
                                    .flatten()
                                    .sum();
                                if scale.weights.probability(weight) >= options.min_probability {
                                    pairs.keep(Ranked {
                                        weight,
                                        left,
                                        right,
                                        score,
                                    });
                                }
                                Ok(())
                            })?;
                        kept.scored += result.candidates;
                        kept.truncated |= result.truncated;
                        kept.approximate |= result.approximate;
                        Ok(())
                    },
                )?;
                Ok(kept)
            },
            Kept::merge,
        )?;

        let explain = |ranked: &Ranked| {
            let comparison = self.explain_pair(ranked.left, ranked.right, ranked.score)?;
            Ok(WeightedMatch {
                left: ranked.left,
                right: ranked.right,
                bits: scale.bits(self, ranked.left, ranked.right, &comparison),
                weight: ranked.weight,
                probability: scale.weights.probability(ranked.weight),
                comparison,
            })
        };
        let ranked = kept.heap.into_sorted_vec();
        let matches = if rayon::current_thread_index().is_some() {
            ranked.par_iter().map(explain).collect::<Result<_>>()?
        } else {
            ranked.iter().map(explain).collect::<Result<_>>()?
        };
        Ok(WeightedReport {
            matches,
            total_matches: kept.total,
            candidates: kept.scored,
            candidate_truncated: kept.truncated,
            approximate: kept.approximate,
            weights: scale.weights,
            fit,
        })
    }

    /// The weights of the index: chance agreement from random pairs,
    /// agreement inside an entity from the candidates of records spread
    /// over `queries`.
    fn fit_weights(&self, queries: &[usize], exhaustive: bool) -> Result<(Weights, Fit)> {
        let chance = self.chance()?;
        let sample: Vec<usize> = queries
            .iter()
            .copied()
            .step_by(queries.len().div_ceil(FIT_QUERIES).max(1))
            .collect();
        let wanted = Candidates {
            lane: PARTIAL_SIMILARITY,
            max_entries: FIT_ENTRIES,
            exhaustive,
            later: false,
            values: true,
        };
        // One row of codes per candidate, in the order of the sample.
        let rows = crate::fold_queries(
            sample.clone(),
            Vec::new,
            |mut rows: Vec<u8>, left| {
                self.visit_candidates(left, wanted, None, &mut |right, score| {
                    rows.extend(codes(&self.explain_pair(left, right, score)?));
                    Ok(())
                })?;
                Ok(rows)
            },
            |mut rows, mut later| {
                rows.append(&mut later);
                rows
            },
        )?;
        // Each sampled record against every other record of its partition.
        let records = self.store().records();
        let compared: usize = sample
            .iter()
            .map(|&slot| {
                self.partition(&records[slot])
                    .iter()
                    .filter(|&&other| self.store().is_active(other as usize))
                    .count()
                    .saturating_sub(1)
            })
            .sum();
        #[allow(clippy::cast_precision_loss)]
        Ok(Weights::fit(&chance, &rows, compared as f64))
    }

    /// `u` of each field: its levels over pairs of one partition taken at
    /// random, or over every pair when there are few.
    fn chance(&self) -> Result<Vec<[f64; LEVELS]>> {
        let fields = self.schema().fields().len();
        let mut partitions: Vec<Vec<u32>> = self
            .partitions
            .values()
            .map(|slots| {
                slots
                    .iter()
                    .copied()
                    .filter(|&slot| self.store().is_active(slot as usize))
                    .collect::<Vec<u32>>()
            })
            .filter(|slots| slots.len() > 1)
            .collect();
        partitions.sort_unstable_by_key(|slots| slots[0]);
        // Pairs before each partition, to draw a partition by its pairs.
        let mut before = Vec::with_capacity(partitions.len());
        let mut pairs = 0usize;
        for slots in &partitions {
            before.push(pairs);
            pairs = pairs.saturating_add(slots.len() * (slots.len() - 1) / 2);
        }
        let count = |counts: &mut Vec<[u64; LEVELS]>, left: u32, right: u32| -> Result<()> {
            if let Some(comparison) = self.compare_pair(left as usize, right as usize)? {
                for (field, compared) in counts.iter_mut().zip(comparison.fields.iter()) {
                    if let Some(similarity) = compared.similarity {
                        field[Level::of(similarity) as usize] += 1;
                    }
                }
            }
            Ok(())
        };
        let batch = |number: usize| -> Result<Vec<[u64; LEVELS]>> {
            let mut counts = vec![[0u64; LEVELS]; fields];
            if pairs <= CHANCE_PAIRS {
                // Few pairs: each partition in turn, every pair of it.
                for slots in partitions.iter().skip(number).step_by(CHANCE_BATCHES) {
                    for (position, &left) in slots.iter().enumerate() {
                        for &right in &slots[position + 1..] {
                            count(&mut counts, left, right)?;
                        }
                    }
                }
                return Ok(counts);
            }
            let mut state = (0x9E37_79B9_7F4A_7C15_u64 ^ ((number as u64 + 1) << 32)) | 1;
            let mut below = |bound: usize| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                usize::try_from(state % bound as u64).unwrap_or(0)
            };
            for _ in 0..CHANCE_PAIRS / CHANCE_BATCHES {
                let drawn = below(pairs);
                let slots = &partitions[before.partition_point(|&start| start <= drawn) - 1];
                let left = below(slots.len());
                let right = (left + 1 + below(slots.len() - 1)) % slots.len();
                count(&mut counts, slots[left], slots[right])?;
            }
            Ok(counts)
        };
        let add = |mut total: Vec<[u64; LEVELS]>, counts: Vec<[u64; LEVELS]>| {
            for (field, counted) in total.iter_mut().zip(counts) {
                for (sum, count) in field.iter_mut().zip(counted) {
                    *sum += count;
                }
            }
            total
        };
        let zero = || vec![[0u64; LEVELS]; fields];
        let counts = if rayon::current_thread_index().is_some() {
            (0..CHANCE_BATCHES)
                .into_par_iter()
                .map(batch)
                .try_reduce(zero, |a, b| Ok(add(a, b)))?
        } else {
            (0..CHANCE_BATCHES)
                .map(batch)
                .try_fold(zero(), |a, b| b.map(|b| add(a, b)))?
        };
        Ok(counts.into_iter().map(FieldWeights::chance).collect())
    }

    fn scale(&self, weights: Weights) -> Scale {
        let fields = self.schema().fields().len();
        let mut scale = Scale {
            weights,
            counts: vec![HashMap::new(); fields],
            valued: vec![0; fields],
        };
        let records = self.store().records();
        for slot in self.store().active_slots() {
            for (field, prepared) in records[slot].fields().iter().enumerate() {
                if let Some(key) = prepared.value_key() {
                    *scale.counts[field].entry(key).or_default() += 1;
                    scale.valued[field] += 1;
                }
            }
        }
        scale
    }
}
