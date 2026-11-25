//! Weights learned from the records themselves, without labels: how much
//! each field's similarity says about two records being one entity.
//!
//! This is the model of probabilistic record linkage (Fellegi and Sunter,
//! 1969). A field similarity is read as one of four [`Level`]s. For each
//! field and level, `m` is how often the level occurs between two records of
//! one entity, and `u` how often it occurs between two records of different
//! entities. A level then weighs `log2(m / u)` bits: positive when it is
//! more common inside an entity than by chance, negative otherwise. The
//! weight of a pair is the sum over its fields, a missing value adding
//! nothing, and with the prior it gives a probability.
//!
//! - **u** is counted over pairs of records taken at random, nearly all of
//!   different entities ([`FieldWeights::chance`]).
//! - **Equal values** use the frequency of the value in place of `u`, when
//!   the caller gives it: sharing a rare value weighs more than sharing a
//!   common one.
//! - **m** comes from candidate pairs, split in two classes by expectation
//!   maximisation ([`Weights::fit`]): the class that agrees gives `m`. The
//!   other class is discarded, because candidates were selected for
//!   agreeing and do not describe unrelated records.
//!
//! Fields are assumed independent given the class, as in the original
//! model: two fields that always agree together are counted twice.

/// Levels of a field similarity.
pub const LEVELS: usize = 4;

/// Similarity from which two values are [`Level::Equal`]: 1 up to rounding.
const EQUAL: f32 = 0.9999;
/// Similarity from which two values are [`Level::Close`].
pub const CLOSE_SIMILARITY: f32 = 0.88;
/// Similarity from which two values are [`Level::Partial`]: below it, a
/// field counts as different.
pub const PARTIAL_SIMILARITY: f32 = 0.7;

/// Lowest probability of a level: a level never seen is not impossible.
const FLOOR: f64 = 1e-4;
/// Observations below which a level borrows the weight of the next one.
const SUPPORT: f64 = 5.0;
/// Rounds of expectation maximisation at most.
const ROUNDS: usize = 100;
/// Largest change of a parameter at which the rounds stop.
const CONVERGED: f64 = 1e-6;

/// How alike the two values of a field are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    Equal = 0,
    Close = 1,
    Partial = 2,
    Different = 3,
}

impl Level {
    pub const ALL: [Self; LEVELS] = [Self::Equal, Self::Close, Self::Partial, Self::Different];

    /// The level of a field similarity in `[0, 1]`.
    pub fn of(similarity: f32) -> Self {
        if similarity >= EQUAL {
            Self::Equal
        } else if similarity >= CLOSE_SIMILARITY {
            Self::Close
        } else if similarity >= PARTIAL_SIMILARITY {
            Self::Partial
        } else {
            Self::Different
        }
    }

    /// The level as stored in a row of [`Weights::fit`]: 1 to 4, 0 being a
    /// field without evidence.
    pub fn code(level: Option<Self>) -> u8 {
        level.map_or(0, |level| level as u8 + 1)
    }

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code).checked_sub(1)?).copied()
    }

    pub fn name(self) -> &'static str {
        ["equal", "close", "partial", "different"][self as usize]
    }
}

/// The weights of one field.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldWeights {
    /// Probability of each level between two records of one entity.
    pub m: [f64; LEVELS],
    /// Probability of each level between two records of different entities.
    pub u: [f64; LEVELS],
    /// Bits each level adds to a pair, `log2(m / u)` where the level was
    /// observed often enough.
    pub bits: [f64; LEVELS],
}

impl FieldWeights {
    /// `u` from the levels counted over random pairs with both values.
    pub fn chance(counts: [u64; LEVELS]) -> [f64; LEVELS] {
        let total: u64 = counts.iter().sum();
        let mut u = [0.0; LEVELS];
        if total == 0 {
            // Nothing observed: no level tells anything.
            return [0.25; LEVELS];
        }
        for (probability, &count) in u.iter_mut().zip(&counts) {
            *probability = (exact(count) + 0.5) / (exact(total) + 0.5 * exact(LEVELS as u64));
        }
        u
    }

    /// The bits a level adds. `frequency` is the share of records holding
    /// the equal value, when known: it then replaces `u` for
    /// [`Level::Equal`], never below what one record in a billion gives.
    pub fn weight(&self, level: Level, frequency: Option<f64>) -> f64 {
        let index = level as usize;
        match (level, frequency) {
            (Level::Equal, Some(frequency)) if self.m[index] > 0.0 => {
                (self.m[index] / frequency.clamp(1e-9, 1.0)).log2()
            }
            _ => self.bits[index],
        }
    }
}

/// The weights of every field of a schema, in schema order, and the prior.
#[derive(Debug, Clone, PartialEq)]
pub struct Weights {
    pub fields: Vec<FieldWeights>,
    /// Bits before any evidence: `log2` of the odds that two records taken
    /// at random are one entity.
    pub prior: f64,
}

/// What [`Weights::fit`] learned beyond the weights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Share of the candidates in the class that agrees.
    pub share: f64,
    pub rounds: usize,
}

impl Weights {
    /// Learns the weights of `fields` fields.
    ///
    /// `chance[f]` is `u` of field `f` ([`FieldWeights::chance`]).
    /// `candidates` holds one row of `fields` codes per candidate pair
    /// ([`Level::code`]). `pairs` is the number of pairs the candidates
    /// were drawn from, for the prior.
    ///
    /// # Panics
    ///
    /// When `chance` or the rows do not have `fields` entries.
    pub fn fit(chance: &[[f64; LEVELS]], candidates: &[u8], pairs: f64) -> (Self, Fit) {
        let fields = chance.len();
        assert!(
            fields > 0 && candidates.len().is_multiple_of(fields),
            "rows of {fields} codes"
        );
        let rows = candidates.len() / fields;
        let classes = Classes::split(fields, candidates);
        let fields = classes
            .agreeing
            .iter()
            .zip(chance)
            .zip(&classes.observed)
            .map(|((&m, &u), observed)| {
                let mut bits = [0.0; LEVELS];
                // From the least similar level up: a level with too few
                // observations of the agreeing class takes the weight of the
                // level below it, never a weight of its own.
                for level in (0..LEVELS).rev() {
                    bits[level] = if observed[level] >= SUPPORT || level == LEVELS - 1 {
                        (m[level] / u[level]).log2()
                    } else {
                        bits[level + 1]
                    };
                }
                FieldWeights { m, u, bits }
            })
            .collect();
        let matches = classes.share * exact(rows as u64);
        let odds = if pairs > matches && matches > 0.0 {
            matches / (pairs - matches)
        } else {
            1e-9
        };
        (
            Self {
                fields,
                prior: odds.max(1e-12).log2(),
            },
            Fit {
                share: classes.share,
                rounds: classes.rounds,
            },
        )
    }

    /// The probability that a pair weighing `bits` in all is one entity.
    pub fn probability(&self, bits: f64) -> f64 {
        1.0 / (1.0 + (-(bits + self.prior)).exp2())
    }
}

/// The candidates as two classes, by expectation maximisation.
struct Classes {
    /// Probability of each level in the class that agrees, by field.
    agreeing: Vec<[f64; LEVELS]>,
    /// Expected rows of that class at each level, by field.
    observed: Vec<[f64; LEVELS]>,
    /// Share of the rows in that class.
    share: f64,
    rounds: usize,
}

impl Classes {
    fn split(fields: usize, candidates: &[u8]) -> Self {
        let rows = candidates.len() / fields;
        let mut m = vec![[0.6, 0.2, 0.12, 0.08]; fields];
        let mut other = vec![[0.15, 0.05, 0.2, 0.6]; fields];
        let mut observed = vec![[0.0f64; LEVELS]; fields];
        let (mut share, mut rounds) = (0.3, 0);
        while rows > 0 && rounds < ROUNDS {
            rounds += 1;
            let mut agree = vec![[0.0f64; LEVELS]; fields];
            let mut differ = vec![[0.0f64; LEVELS]; fields];
            let mut matched = 0.0;
            for row in candidates.chunks_exact(fields) {
                let (mut same, mut distinct) = (share, 1.0 - share);
                for (field, &code) in row.iter().enumerate() {
                    if let Some(level) = Level::from_code(code) {
                        same *= m[field][level as usize];
                        distinct *= other[field][level as usize];
                    }
                }
                let probability = if same + distinct > 0.0 {
                    same / (same + distinct)
                } else {
                    0.0
                };
                matched += probability;
                for (field, &code) in row.iter().enumerate() {
                    if let Some(level) = Level::from_code(code) {
                        agree[field][level as usize] += probability;
                        differ[field][level as usize] += 1.0 - probability;
                    }
                }
            }
            let previous = (share, m.clone());
            share = (matched / exact(rows as u64)).clamp(1e-6, 1.0 - 1e-6);
            for (table, counts) in m
                .iter_mut()
                .zip(&agree)
                .chain(other.iter_mut().zip(&differ))
            {
                let evidence: f64 = counts.iter().sum();
                if evidence > 0.0 {
                    for (probability, count) in table.iter_mut().zip(counts) {
                        *probability = (count / evidence).max(FLOOR);
                    }
                }
            }
            observed = agree;
            let moved = m
                .iter()
                .flatten()
                .zip(previous.1.iter().flatten())
                .map(|(new, old)| (new - old).abs())
                .fold((share - previous.0).abs(), f64::max);
            if moved < CONVERGED {
                break;
            }
        }
        // The class that agrees is the one that differs less.
        let differs = |class: &[[f64; LEVELS]]| -> f64 {
            class
                .iter()
                .map(|levels| levels[Level::Different as usize])
                .sum()
        };
        if differs(&m) > differs(&other) {
            m = other;
            share = 1.0 - share;
            observed = vec![[SUPPORT; LEVELS]; fields];
        }
        Self {
            agreeing: m,
            observed,
            share,
            rounds,
        }
    }
}

/// A count as a float; counts here stay far below 2^52.
#[allow(clippy::cast_precision_loss)]
fn exact(count: u64) -> f64 {
    count as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows of `fields` codes drawn from two classes, from a seed: a share
    /// of pairs of one entity, which mostly agree, and others, which
    /// mostly differ; one value in ten is missing.
    fn candidates(rows: usize, fields: usize, share: f64, seed: u64) -> (Vec<u8>, Vec<bool>) {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            #[allow(clippy::cast_precision_loss)]
            let value = (state % 1_000_000) as f64 / 1e6;
            value
        };
        let draw = |levels: [f64; LEVELS], at: f64| {
            let mut total = 0.0;
            for (level, probability) in levels.iter().enumerate() {
                total += probability;
                if at < total {
                    return u8::try_from(level + 1).unwrap();
                }
            }
            4
        };
        let (mut codes, mut same) = (Vec::new(), Vec::new());
        for _ in 0..rows {
            let entity = next() < share;
            same.push(entity);
            for _ in 0..fields {
                let levels = if entity {
                    [0.7, 0.15, 0.1, 0.05]
                } else {
                    [0.05, 0.05, 0.2, 0.7]
                };
                codes.push(if next() < 0.1 {
                    0
                } else {
                    draw(levels, next())
                });
            }
        }
        (codes, same)
    }

    const CHANCE: [f64; LEVELS] = [0.001, 0.001, 0.01, 0.988];

    #[test]
    fn levels_follow_their_limits() {
        for (similarity, level) in [
            (1.0, Level::Equal),
            (0.9999, Level::Equal),
            (0.99, Level::Close),
            (0.88, Level::Close),
            (0.879, Level::Partial),
            (0.7, Level::Partial),
            (0.699, Level::Different),
            (0.0, Level::Different),
        ] {
            assert_eq!(Level::of(similarity), level, "{similarity}");
        }
        for level in Level::ALL {
            assert_eq!(Level::from_code(Level::code(Some(level))), Some(level));
        }
        assert_eq!(Level::code(None), 0);
        assert_eq!(Level::from_code(0), None);
        assert_eq!(Level::from_code(5), None);
    }

    #[test]
    fn fitting_recovers_the_class_that_agrees() {
        for (share, seed) in [(0.2, 1), (0.5, 2), (0.8, 3)] {
            let (codes, _) = candidates(20_000, 5, share, seed);
            let (weights, fit) = Weights::fit(&[CHANCE; 5], &codes, 1e9);
            assert!((fit.share - share).abs() < 0.03, "{share}: {}", fit.share);
            assert!(fit.rounds < ROUNDS, "{share}: no convergence");
            for field in &weights.fields {
                for (found, expected) in field.m.iter().zip([0.7, 0.15, 0.1, 0.05]) {
                    assert!((found - expected).abs() < 0.03, "{share}: {:?}", field.m);
                }
                // Agreement adds bits, a difference removes some.
                assert!(field.bits[0] > 5.0 && field.bits[1] > 5.0 && field.bits[2] > 2.0);
                assert!(field.bits[3] < -3.0, "{:?}", field.bits);
            }
        }
    }

    #[test]
    fn weights_rank_one_entity_above_the_rest() {
        let (codes, same) = candidates(20_000, 5, 0.3, 7);
        let (weights, _) = Weights::fit(&[CHANCE; 5], &codes, 1e9);
        let mut ranked: Vec<(f64, bool)> = codes
            .as_chunks::<5>()
            .0
            .iter()
            .zip(&same)
            .map(|(row, &entity)| {
                let bits = row
                    .iter()
                    .zip(&weights.fields)
                    .filter_map(|(&code, field)| Some(field.weight(Level::from_code(code)?, None)))
                    .sum();
                (bits, entity)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        // As many rows as there are pairs of one entity, from the top: the
        // other class agrees far more than chance, so its bits are positive
        // too, yet it ranks below.
        let entities = same.iter().filter(|&&entity| entity).count();
        let found = ranked[..entities]
            .iter()
            .filter(|(_, entity)| *entity)
            .count();
        assert!(found * 100 >= entities * 95, "{found} of {entities}");
    }

    #[test]
    fn a_rare_value_weighs_more_than_a_common_one() {
        let (codes, _) = candidates(5_000, 2, 0.4, 11);
        let (weights, _) = Weights::fit(&[CHANCE; 2], &codes, 1e9);
        let field = &weights.fields[0];
        let (rare, common) = (
            field.weight(Level::Equal, Some(1e-5)),
            field.weight(Level::Equal, Some(0.2)),
        );
        assert!(
            rare > field.bits[0] && field.bits[0] > common,
            "{rare} {common}"
        );
        // The frequency only concerns equal values, and stays bounded.
        assert_eq!(field.weight(Level::Close, Some(1e-5)), field.bits[1]);
        assert!(field.weight(Level::Equal, Some(0.0)).is_finite());
    }

    #[test]
    fn a_level_never_seen_takes_the_weight_below_it() {
        // No close value in either class: without the rule, two floors
        // would give the level a weight of its own.
        let codes: Vec<u8> = (0..4_000u32)
            .flat_map(|row| if row % 4 == 0 { [1, 1] } else { [4, 4] })
            .collect();
        let chance = FieldWeights::chance([10, 0, 100, 99_890]);
        let (weights, _) = Weights::fit(&[chance; 2], &codes, 1e9);
        for field in &weights.fields {
            assert_eq!(field.bits[1], field.bits[2]);
            assert_eq!(field.bits[2], field.bits[3]);
            assert!(
                field.bits[0] > 8.0 && field.bits[3] < 0.0,
                "{:?}",
                field.bits
            );
        }
    }

    #[test]
    fn chance_is_smoothed_and_sums_to_one() {
        let u = FieldWeights::chance([0, 3, 97, 999_900]);
        assert!((u.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(u[0] > 0.0 && u[0] < u[1] && u[1] < u[2] && u[2] < u[3]);
        assert_eq!(FieldWeights::chance([0; LEVELS]), [0.25; LEVELS]);
    }

    #[test]
    fn probability_grows_with_the_bits_around_the_prior() {
        let (codes, _) = candidates(2_000, 3, 0.5, 5);
        let (weights, fit) = Weights::fit(&[CHANCE; 3], &codes, 1e6);
        // About 1,000 matching pairs among a million: ten bits against.
        assert!(
            (weights.prior + 10.0).abs() < 0.5,
            "{} {fit:?}",
            weights.prior
        );
        assert!((weights.probability(-weights.prior) - 0.5).abs() < 1e-12);
        let mut previous = 0.0;
        for bits in [-40.0, -10.0, 0.0, 5.0, 10.0, 20.0, 60.0] {
            let probability = weights.probability(bits);
            assert!((0.0..=1.0).contains(&probability) && probability >= previous);
            previous = probability;
        }
        assert!(weights.probability(60.0) > 0.999_999);
    }

    #[test]
    fn nothing_to_learn_from_gives_neutral_fields() {
        let (weights, fit) = Weights::fit(&[[0.25; LEVELS]; 2], &[], 0.0);
        assert_eq!(fit.rounds, 0);
        assert!(weights.prior < -20.0);
        // Every value missing: the starting point is kept, and stays finite.
        let (weights, _) = Weights::fit(&[CHANCE; 2], &[0; 200], 1e6);
        assert!(
            weights
                .fields
                .iter()
                .flat_map(|field| field.bits)
                .all(f64::is_finite)
        );
    }
}
