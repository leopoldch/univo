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
//!    [`FIT_QUERIES`] records drawn over the index, at most [`FIT_PAIRS`]
//!    each, with a fixed seed: the order of the records does not choose
//!    which pairs are learned from.
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
use std::collections::{BTreeMap, BinaryHeap, HashMap};

use rayon::prelude::*;
use univo_core::{
    CLOSE_SIMILARITY, Comparison, Error, FieldKind, FieldState, FieldWeights, Fit, LEVELS, Level,
    NormalizedValue, PARTIAL_SIMILARITY, Result, Weights,
};

use crate::SearchIndex;
use crate::search::{Candidates, Draws, one_in};

/// Pairs taken at random to count chance agreement.
const CHANCE_PAIRS: usize = 1_000_000;

/// Batches the random pairs are drawn in, each with its own seed, so that
/// the counts do not depend on the threads.
const CHANCE_BATCHES: usize = 64;

/// Records whose candidates the weights are learned from, at most.
pub const FIT_QUERIES: usize = 1024;

/// Index entries one lane reads for one of these records.
const FIT_ENTRIES: usize = 4096;

/// Candidates one of these records gives at most, drawn among all of them.
/// With [`FIT_QUERIES`] it bounds what is learned from: about a million
/// pairs, kept as counts of their distinct combinations of levels.
pub const FIT_PAIRS: usize = 1024;

/// Seed of the draw of the records the weights are learned from.
const FIT_SEED: u64 = 0xF175_EED0_F7E5_7A11;

/// Seed of the pairs taken at random, one sequence per batch.
const CHANCE_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

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
    /// weighs by how common it is, the same for both records: the bits do
    /// not depend on which one is on the left.
    fn field_bits(
        &self,
        index: &SearchIndex,
        field: usize,
        (left, right): (usize, usize),
        similarity: f32,
    ) -> f64 {
        let level = level(index, field, left, right, similarity);
        let share = (level == Level::Equal)
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
            let best = valued[field].map_or(0.0, |(_, share)| {
                self.at_best(field, share, is_object(index, field, slot))
            });
            if spent - left_out[field] + best < needed - 1e-9 {
                spent += best - left_out[field];
                lanes[field] = f32::INFINITY;
            }
        }
    }

    /// The most bits `field` adds to a pair that agrees on it, for a record
    /// whose value is `share` of the others: those of the same value, or
    /// of a close or partial likeness where the field can give one. A text
    /// can, and an `object` compared key by key; any other exact value is
    /// equal or different.
    fn at_best(&self, field: usize, share: f64, object: bool) -> f64 {
        let learned = &self.weights.fields[field];
        let mut best = learned.weight(Level::Equal, Some(share)).max(0.0);
        if !self.exact[field] || object {
            best = best
                .max(learned.bits[Level::Close as usize])
                .max(learned.bits[Level::Partial as usize]);
        }
        best
    }
}

/// The level of `field` between two stored records at `similarity`: equal
/// only when both hold the same value ([`Level::of`]).
fn level(index: &SearchIndex, field: usize, left: usize, right: usize, similarity: f32) -> Level {
    Level::of(similarity, || {
        let records = index.store().records();
        records[left].fields()[field].value_key() == records[right].fields()[field].value_key()
    })
}

/// Whether the record at `slot` holds an object in `field`.
fn is_object(index: &SearchIndex, field: usize, slot: usize) -> bool {
    matches!(
        index.store().records()[slot].fields()[field].state,
        FieldState::Exact(NormalizedValue::Map(_))
    )
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
            at_most: usize::MAX,
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
                        let (result, _) = result;
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
    /// agreement inside an entity from the candidates of records drawn
    /// over `queries`.
    fn fit_weights(&self, queries: &[usize], exhaustive: bool) -> Result<(Weights, Fit)> {
        let chance = self.chance()?;
        let fields = self.schema().fields().len();
        let sample: Vec<usize> = one_in(
            queries.iter().copied(),
            queries.len().div_ceil(FIT_QUERIES),
            FIT_SEED,
        )
        .collect();
        let wanted = Candidates {
            lane: PARTIAL_SIMILARITY,
            max_entries: FIT_ENTRIES,
            exhaustive,
            later: false,
            at_most: FIT_PAIRS,
        };
        // The candidates as counts of their combinations of levels, with
        // how many were proposed and how many of those were kept.
        let learned = crate::fold_queries(
            sample.clone(),
            || (BTreeMap::<Box<[u8]>, u64>::new(), 0usize, 0usize),
            |(mut combinations, mut proposed, mut kept), left| {
                let mut row = vec![0u8; fields];
                let (_, found) =
                    self.visit_candidates(left, wanted, None, None, &mut |right, _| {
                        row.fill(0);
                        self.similarities(left, right, &mut |field, similarity| {
                            row[field] = Level::code(
                                similarity
                                    .map(|similarity| level(self, field, left, right, similarity)),
                            );
                        })?;
                        match combinations.get_mut(row.as_slice()) {
                            Some(count) => *count += 1,
                            None => {
                                combinations.insert(row.clone().into_boxed_slice(), 1);
                            }
                        }
                        Ok(())
                    })?;
                proposed += found.proposed;
                kept += found.kept;
                Ok((combinations, proposed, kept))
            },
            |(mut combinations, proposed, kept), (others, more, held)| {
                for (row, count) in others {
                    *combinations.entry(row).or_default() += count;
                }
                (combinations, proposed + more, kept + held)
            },
        )?;
        let (combinations, proposed, kept) = learned;
        let rows: Vec<u8> = combinations
            .keys()
            .flat_map(|row| row.iter().copied())
            .collect();
        let counts: Vec<u64> = combinations.values().copied().collect();
        // Each sampled record against every other record of its partition,
        // less the share of its candidates that was not kept.
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
        let pairs = compared as f64 * (kept.max(1) as f64 / proposed.max(1) as f64);
        Ok(Weights::fit(&chance, &rows, &counts, pairs))
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
                let fields = counts.iter_mut().zip(comparison.fields.iter());
                for (position, (counted, compared)) in fields.enumerate() {
                    if let Some(similarity) = compared.similarity {
                        // Counted in order: `field` is the position.
                        counted[level(self, position, left as usize, right as usize, similarity)
                            as usize] += 1;
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
            let mut random = Draws::new(CHANCE_SEED ^ number as u64);
            for _ in 0..CHANCE_PAIRS / CHANCE_BATCHES {
                let drawn = random.below(pairs);
                let slots = &partitions[before.partition_point(|&start| start <= drawn) - 1];
                let left = random.below(slots.len());
                let right = (left + 1 + random.below(slots.len() - 1)) % slots.len();
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

#[cfg(test)]
mod tests {
    use univo_core::{Exact, FieldPath, Map, Schema, Text, Value, prepare};

    use super::*;

    fn path(dotted: &str) -> FieldPath {
        FieldPath::parse(dotted).unwrap()
    }

    /// A name, a code and a contact object. Most contacts are the same
    /// `{"city": "lyon"}`; one record in four adds a phone of its own, and
    /// names come in pairs, so that a common object and a rarer one that
    /// agrees with it on every shared key meet in many pairs.
    fn index(records: i64) -> SearchIndex {
        let schema = Schema::builder()
            .id_field(path("id"))
            .text(path("name"), Text::default())
            .exact(path("code"), Exact::default())
            .exact(path("contact"), Exact::default())
            .build()
            .unwrap();
        let prepared = (0..records)
            .map(|id| {
                let mut contact = vec![("city".to_owned(), Value::Str("lyon".into()))];
                if id % 4 == 0 {
                    contact.push(("phone".to_owned(), Value::Int(id)));
                }
                let entries = vec![
                    ("id".to_owned(), Value::Int(id)),
                    ("name".to_owned(), Value::Str(format!("person {}", id / 2))),
                    (
                        "code".to_owned(),
                        Value::Str(["a", "b", "c"][usize::try_from(id % 3).unwrap()].into()),
                    ),
                    ("contact".to_owned(), Value::Map(Map::new(contact).unwrap())),
                ];
                prepare(&schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
            })
            .collect();
        let mut index = SearchIndex::new(schema, 10_000, None).unwrap();
        index.add(prepared).unwrap();
        index
    }

    #[test]
    fn a_pair_weighs_the_same_whichever_record_comes_first() {
        let index = index(240);
        let queries: Vec<usize> = index.store().active_slots().collect();
        let (weights, _) = index.fit_weights(&queries, false).unwrap();
        let scale = index.scale(weights);
        let (mut objects, mut equal) = (0, 0);
        for left in 0..240 {
            for right in left + 1..240 {
                let forward = scale.bits(&index, left, right).unwrap();
                let backward = scale.bits(&index, right, left).unwrap();
                assert_eq!(forward, backward, "{left} {right}");
                // The contact is the third field: shared keys that agree
                // make two different objects close, never equal.
                let same = is_object(&index, 2, left)
                    && index.store().records()[left].fields()[2].value_key()
                        == index.store().records()[right].fields()[2].value_key();
                let found = level(&index, 2, left, right, 1.0);
                assert_eq!(found == Level::Equal, same);
                objects += usize::from(found == Level::Close);
                equal += usize::from(same);
            }
        }
        assert!(objects > 5_000 && equal > 5_000, "{objects} {equal}");
    }

    #[test]
    fn an_object_field_is_bounded_by_its_partial_levels_too() {
        let index = index(240);
        let field = |bits: [f64; LEVELS]| FieldWeights {
            m: [0.25; LEVELS],
            u: [0.25; LEVELS],
            bits,
        };
        // Closeness weighs more than an equal common value, for the code
        // and for the contact alike; the name weighs nothing.
        let learned = [1.0, 9.0, 6.0, -2.0];
        let scale = index.scale(Weights {
            fields: vec![field([0.0; LEVELS]), field(learned), field(learned)],
            prior: -10.0,
        });
        // A code is equal or different; a contact can be close.
        let common = scale.common(&index, 2, 1).unwrap().1;
        assert!(scale.at_best(1, common, false) < 2.0);
        assert!((scale.at_best(2, common, true) - 9.0).abs() < 1e-12);
        let mut lanes = Vec::new();
        scale.lanes(&index, 1, 5.0, &mut lanes);
        assert!(
            lanes[1].is_infinite(),
            "an equal code cannot reach 5 bits: {lanes:?}"
        );
        assert!(lanes[2].is_finite(), "a close contact can: {lanes:?}");
        // With room for both, both are left out.
        scale.lanes(&index, 1, 20.0, &mut lanes);
        assert!(
            lanes[1].is_infinite() && lanes[2].is_infinite(),
            "{lanes:?}"
        );
    }

    #[test]
    fn learning_keeps_counts_and_a_bounded_sample() {
        // Every record equal to every other: each of the 1,000 sampled
        // records has 2,000 candidates, itself included, and gives one in
        // two of them, all of one combination of levels.
        let schema = Schema::builder()
            .text(path("a"), Text::default())
            .text(path("b"), Text::default())
            .build()
            .unwrap();
        let entries = || {
            vec![
                ("a".to_owned(), Value::Str("same".into())),
                ("b".to_owned(), Value::Str("as ever".into())),
            ]
        };
        let prepared = (0..2000)
            .map(|_| prepare(&schema, &Value::Map(Map::new(entries()).unwrap())).unwrap())
            .collect();
        let mut index = SearchIndex::new(schema, 10_000, None).unwrap();
        index.add(prepared).unwrap();
        let queries: Vec<usize> = index.store().active_slots().collect();
        let (weights, fit) = index.fit_weights(&queries, false).unwrap();
        // One fewer where the record itself was among those kept.
        assert!((1000 * 999..=1000 * 1000).contains(&fit.rows), "{fit:?}");
        assert!(fit.rows <= FIT_QUERIES * FIT_PAIRS);
        // One combination cannot be split in two classes, so the share is
        // whatever the first rounds left. What was not kept still counts in
        // the prior: most pairs match, and the odds are in favour.
        assert!(weights.prior > 1.0, "{fit:?} {}", weights.prior);
    }

    /// 2,048 records of one city, stored as 1,024 pairs of duplicates next
    /// to each other, each pair with three codes of its own.
    fn neighbours() -> SearchIndex {
        let schema = Schema::builder()
            .text(path("city"), Text::default())
            .exact(path("a"), Exact::default())
            .exact(path("b"), Exact::default())
            .exact(path("c"), Exact::default())
            .build()
            .unwrap();
        let prepared = (0..2048i64)
            .map(|slot| {
                let mut entries = vec![("city".to_owned(), Value::Str("lyon".into()))];
                for (code, base) in [("a", 0), ("b", 10_000), ("c", 20_000)] {
                    entries.push((code.to_owned(), Value::Int(base + slot / 2)));
                }
                prepare(&schema, &Value::Map(Map::new(entries).unwrap())).unwrap()
            })
            .collect();
        let mut index = SearchIndex::new(schema, 10_000, None).unwrap();
        index.add(prepared).unwrap();
        index
    }

    #[test]
    fn duplicates_stored_next_to_each_other_are_learned_from() {
        // One record in two is a query and gives one in two of its 2,048
        // candidates. Taken at fixed places, both keep the even slots and
        // no query ever meets its duplicate, at the next odd slot.
        let index = neighbours();
        let queries: Vec<usize> = index.store().active_slots().collect();
        let (weights, fit) = index.fit_weights(&queries, false).unwrap();
        // One query in two keeps its duplicate: about 512 of the rows.
        #[allow(clippy::cast_precision_loss)]
        let agreeing = fit.share * fit.rows as f64;
        assert!((400.0..=624.0).contains(&agreeing), "{agreeing} {fit:?}");
        for codes in &weights.fields[1..] {
            assert!(codes.m[Level::Equal as usize] > 0.99, "{codes:?}");
            assert!(codes.bits[Level::Different as usize] < -5.0, "{codes:?}");
        }

        let report = index
            .find_weighted(
                WeightedOptions {
                    min_probability: 0.9,
                    limit: usize::MAX,
                    max_entries: usize::MAX,
                    every_lane: false,
                },
                false,
                usize::MAX,
                usize::MAX,
            )
            .unwrap();
        assert_eq!(report.total_matches, 1024);
        assert!(
            report
                .matches
                .iter()
                .all(|found| found.left % 2 == 0 && found.right == found.left + 1)
        );
        // The prior counts what was not kept: a pair in about 2,047.
        assert!(
            (-11.5..=-10.5).contains(&report.weights.prior),
            "{}",
            report.weights.prior
        );
    }

    #[test]
    fn the_sample_is_the_same_in_exhaustive_mode() {
        // Without lanes, every record of the partition is a candidate: the
        // same draw, so the same duplicates are met.
        let index = neighbours();
        let queries: Vec<usize> = index.store().active_slots().collect();
        let lanes = index.fit_weights(&queries, false).unwrap();
        let exhaustive = index.fit_weights(&queries, true).unwrap();
        assert_eq!(lanes, exhaustive);
    }
}
