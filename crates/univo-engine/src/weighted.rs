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
//! A common value, or a loose likeness, proposes many pairs. In the second
//! pass, each record leaves out the partial likenesses of some fields and
//! the lanes of its most common values, as long as what is left out could
//! not bring a pair to the minimum probability: a pair that reaches it then
//! agrees on something else too, whose lane proposes it. No pair that
//! reaches the minimum is lost to this
//! ([`Scale::lanes`]).
//!
//! Vetoes and partitions apply as in any search: a vetoed pair is not a
//! candidate.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use rayon::prelude::*;
use univo_core::{
    CLOSE_SIMILARITY, Comparison, Error, FieldKind, FieldWeights, Fit, LEVELS, Level,
    PARTIAL_SIMILARITY, Result, Weights,
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
    /// Read every lane of every record from [`PARTIAL_SIMILARITY`], leaving
    /// nothing out. For tests and measurements: the report does not depend
    /// on it.
    pub every_lane: bool,
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
    /// Fields compared by meaning: two texts can be equal for the model
    /// without being the same value.
    semantic: Vec<bool>,
    /// Fields compared by equality.
    exact: Vec<bool>,
    /// Bits the fields of a pair can add while all different.
    different: f64,
}

impl Scale {
    /// How many active records hold the value of `field` in the record at
    /// `slot`, and the share of the other records it is; `None` without a
    /// value.
    fn common(&self, index: &SearchIndex, field: usize, slot: usize) -> Option<(u32, f64)> {
        let key = index.store().records()[slot].fields()[field].value_key()?;
        let (count, valued) = (*self.counts[field].get(&key)?, self.valued[field]);
        let others = f64::from(valued.saturating_sub(1).max(1));
        let share =
            (f64::from(count.saturating_sub(1)) / others).max(1.0 / f64::from(valued.max(1)));
        Some((count, share))
    }

    /// The bits `field` adds to the pair at `similarity`. An equal value
    /// weighs by how common the left record's value is; for a field
    /// compared by meaning, only when both records hold that value.
    fn field_bits(
        &self,
        index: &SearchIndex,
        field: usize,
        (left, right): (usize, usize),
        similarity: f32,
    ) -> f64 {
        let level = Level::of(similarity);
        let records = index.store().records();
        let same = level == Level::Equal
            && (!self.semantic[field]
                || records[left].fields()[field].value_key()
                    == records[right].fields()[field].value_key());
        let share = same
            .then(|| self.common(index, field, left))
            .flatten()
            .map(|(_, share)| share);
        self.weights.fields[field].weight(level, share)
    }

    /// The bits each field of the pair adds, in schema order.
    fn bits(&self, index: &SearchIndex, left: usize, right: usize) -> Result<Box<[Option<f64>]>> {
        let mut bits = vec![None; self.weights.fields.len()];
        index.similarities(left, right, &mut |field, similarity| {
            bits[field] = similarity
                .map(|similarity| self.field_bits(index, field, (left, right), similarity));
        })?;
        Ok(bits.into_boxed_slice())
    }

    /// The sum of [`Self::bits`], without allocating.
    fn weight(&self, index: &SearchIndex, left: usize, right: usize) -> Result<f64> {
        let mut weight = 0.0;
        index.similarities(left, right, &mut |field, similarity| {
            if let Some(similarity) = similarity {
                weight += self.field_bits(index, field, (left, right), similarity);
            }
        })?;
        Ok(weight)
    }

    /// The similarity from which the lanes of each field propose candidates
    /// for the record at `slot`, into `lanes`, so that every pair weighing
    /// at least `needed` bits is still proposed.
    ///
    /// A pair that no lane proposes agrees only on what was left out: a
    /// partial likeness where the lane starts at [`CLOSE_SIMILARITY`], any
    /// agreement where the field is left out whole. It weighs at most the
    /// bits left out, plus what different fields can add. Partial
    /// likenesses are left out first, the lightest first, then whole
    /// fields, the most common values first, as long as that sum stays
    /// below `needed`.
    fn lanes(&self, index: &SearchIndex, slot: usize, needed: f64, lanes: &mut Vec<f32>) {
        let fields = self.weights.fields.len();
        lanes.clear();
        lanes.resize(fields, PARTIAL_SIMILARITY);
        let valued: Vec<Option<(u32, f64)>> = (0..fields)
            .map(|field| self.common(index, field, slot))
            .collect();
        let mut left_out = vec![0.0f64; fields];
        let mut spent = self.different;

        let mut partial: Vec<(f64, usize)> = (0..fields)
            .filter(|&field| valued[field].is_some() && !self.exact[field])
            .map(|field| {
                let bits = self.weights.fields[field].bits[Level::Partial as usize];
                (bits.max(0.0), field)
            })
            .collect();
        partial.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (bits, field) in partial {
            if spent + bits < needed - 1e-9 {
                spent += bits;
                left_out[field] = bits;
                lanes[field] = CLOSE_SIMILARITY;
            }
        }

        let mut whole: Vec<(u32, usize)> = (0..fields)
            .map(|field| (valued[field].map_or(u32::MAX, |(count, _)| count), field))
            .collect();
        whole.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, field) in whole {
            // Without a value, the lanes of the field propose nothing.
            let best = valued[field].map_or(0.0, |(_, share)| self.at_best(field, share));
            if spent - left_out[field] + best < needed - 1e-9 {
                spent += best - left_out[field];
                lanes[field] = f32::INFINITY;
            }
        }
    }

    /// The most bits `field` adds to a pair that agrees on it, for a record
    /// whose value is `share` of the others.
    fn at_best(&self, field: usize, share: f64) -> f64 {
        let learned = &self.weights.fields[field];
        let equal = learned.weight(Level::Equal, Some(share));
        let mut best = equal.max(0.0);
        if !self.exact[field] {
            best = best
                .max(learned.bits[Level::Close as usize])
                .max(learned.bits[Level::Partial as usize]);
        }
        if self.semantic[field] {
            best = best.max(learned.bits[Level::Equal as usize]);
        }
        best
    }
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
        let wanted = Candidates {
            lane: PARTIAL_SIMILARITY,
            max_entries: options.max_entries,
            exhaustive,
            later: true,
        };
        let kept = crate::fold_queries(
            self.query_blocks(&queries),
            || (Kept::new(options.limit), Vec::new()),
            |(mut kept, mut lanes), block| {
                self.for_each_in_block(
                    &block,
                    PARTIAL_SIMILARITY,
                    exhaustive,
                    &sketches,
                    &mut |left, dense| {
                        if !options.every_lane {
                            scale.lanes(self, left, needed, &mut lanes);
                        }
                        let pairs = &mut kept;
                        let result = self.visit_candidates(
                            left,
                            wanted,
                            (!options.every_lane).then_some(lanes.as_slice()),
                            dense,
                            &mut |right, score| {
                                let weight = scale.weight(self, left, right)?;
                                if scale.weights.probability(weight) >= options.min_probability {
                                    pairs.keep(Ranked {
                                        weight,
                                        left,
                                        right,
                                        score,
                                    });
                                }
                                Ok(())
                            },
                        )?;
                        kept.scored += result.candidates;
                        kept.truncated |= result.truncated;
                        kept.approximate |= result.approximate;
                        Ok(())
                    },
                )?;
                Ok((kept, lanes))
            },
            |(kept, lanes), (other, _)| (kept.merge(other), lanes),
        )?
        .0;

        let explain = |ranked: &Ranked| {
            Ok(WeightedMatch {
                left: ranked.left,
                right: ranked.right,
                bits: scale.bits(self, ranked.left, ranked.right)?,
                weight: ranked.weight,
                probability: scale.weights.probability(ranked.weight),
                comparison: self.explain_pair(ranked.left, ranked.right, ranked.score)?,
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
        let fields = self.schema().fields().len();
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
        };
        // One row of codes per candidate, in the order of the sample.
        let rows = crate::fold_queries(
            sample.clone(),
            Vec::new,
            |mut rows: Vec<u8>, left| {
                self.visit_candidates(left, wanted, None, None, &mut |right, _| {
                    let start = rows.len();
                    rows.resize(start + fields, 0);
                    self.similarities(left, right, &mut |field, similarity| {
                        rows[start + field] = Level::code(similarity.map(Level::of));
                    })
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
        let different = weights
            .fields
            .iter()
            .map(|field| field.bits[Level::Different as usize].max(0.0))
            .sum();
        let mut scale = Scale {
            weights,
            counts: vec![HashMap::new(); fields],
            valued: vec![0; fields],
            semantic: (0..fields)
                .map(|field| self.semantic.contains(&field))
                .collect(),
            exact: self
                .schema()
                .fields()
                .iter()
                .map(|field| matches!(field.kind, FieldKind::Exact(_)))
                .collect(),
            different,
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
