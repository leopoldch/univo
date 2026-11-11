//! Semantic similarity of embeddings, as defined in `docs/reference/scoring.md`.

use wide::{CmpGe, f32x4};

/// Lanes summed independently, so that the compiler vectorizes the loop
/// with the instructions every supported CPU has (SSE2, NEON) and no
/// runtime feature detection. Multiplications and additions stay separate:
/// `mul_add` is a slow library call on CPUs without fused multiply-add.
const LANES: usize = 8;

/// Dot product of two vectors of the same length.
///
/// Summation runs over eight independent lanes, then adds them in a fixed
/// order: the result is deterministic for given inputs, on every platform.
pub fn dot(left: &[f32], right: &[f32]) -> f32 {
    debug_assert_eq!(left.len(), right.len());
    let mut sums = [0.0f32; LANES];
    let (left_chunks, left_rest) = left.as_chunks::<LANES>();
    let (right_chunks, right_rest) = right.as_chunks::<LANES>();
    for (a, b) in left_chunks.iter().zip(right_chunks) {
        for lane in 0..LANES {
            sums[lane] += a[lane] * b[lane];
        }
    }
    let mut total =
        ((sums[0] + sums[4]) + (sums[1] + sums[5])) + ((sums[2] + sums[6]) + (sums[3] + sums[7]));
    for (a, b) in left_rest.iter().zip(right_rest) {
        total += a * b;
    }
    total
}

/// Dot products of `N` queries with one row, each bit for bit equal to
/// [`dot`]: every lane sums the same products in the same order, and the
/// lanes are added in the same order. Reading the row once for several
/// queries divides memory traffic by `N`, and the independent sums keep the
/// arithmetic units busy.
///
/// The lanes are explicit four-wide vectors (`wide`, NEON or SSE2, plain
/// code elsewhere): left to itself, the compiler vectorizes across queries
/// with shuffles and runs this two to three times slower. Multiplications
/// and additions stay separate, as in [`dot`].
///
/// # Panics
///
/// When a query and the row differ in length.
pub fn dots<const N: usize>(queries: [&[f32]; N], row: &[f32]) -> [f32; N] {
    let mut blocked = Blocked::new(queries, row);
    blocked.accumulate(blocked.chunks());
    blocked.totals()
}

/// [`dots`], or `None` when none of the queries can reach `threshold`,
/// which is decided after the first `prefix` lanes chunks of the row. With
/// no prefix, the bound is not computed and the result is [`dots`].
///
/// The dot product of the remaining dimensions is at most the product of
/// their norms (Cauchy–Schwarz), given as `rest_norms` for the queries and
/// `row_rest_norm` for the row. A margin of `4 · n · ε` covers the rounding
/// of an `n`-term float sum of unit vectors, so a query whose dot product
/// reaches `threshold` is never left out. Otherwise the products continue
/// from the same partial sums: the result is bit for bit that of [`dots`].
///
/// # Panics
///
/// As [`dots`].
pub fn dots_reaching<const N: usize>(
    queries: [&[f32]; N],
    rest_norms: [f32; N],
    row: &[f32],
    row_rest_norm: f32,
    prefix: usize,
    threshold: f32,
) -> Option<[f32; N]> {
    let mut blocked = Blocked::new(queries, row);
    let prefix = prefix.min(blocked.chunks());
    if prefix > 0 {
        blocked.accumulate(prefix);
        let margin = rounding_margin(row.len());
        let reachable = blocked
            .sums
            .iter()
            .zip(rest_norms)
            .any(|([low, high], rest)| {
                let (low, high) = (low.to_array(), high.to_array());
                let partial = ((low[0] + high[0]) + (low[1] + high[1]))
                    + ((low[2] + high[2]) + (low[3] + high[3]));
                partial + rest * row_rest_norm + margin >= threshold
            });
        if !reachable {
            return None;
        }
    }
    blocked.accumulate(blocked.chunks());
    Some(blocked.totals())
}

/// What the bounds on a dot product of unit vectors of `dimension` values
/// allow for rounding: `4 · n · ε`, several times the error of an `n`-term
/// float sum.
#[allow(clippy::cast_precision_loss)]
pub fn rounding_margin(dimension: usize) -> f32 {
    4.0 * dimension as f32 * f32::EPSILON
}

/// Vectors a [`SketchGroup`] holds.
pub const SKETCH_GROUP: usize = 8;

/// The sketches of up to eight unit vectors, stored dimension by dimension
/// so that one pass over a row's sketch bounds its dot product with all of
/// them, without a horizontal sum.
///
/// A sketch of a vector `v` is its coordinates `Q·v` on a few orthonormal
/// directions `Q`, with the norm of what they leave out, `rest`. For two
/// vectors, `a·b = (Q·a)·(Q·b) + a⊥·b⊥ ≤ (Q·a)·(Q·b) + rest(a)·rest(b)`
/// (Cauchy–Schwarz), whatever the directions: they only decide how tight
/// the bound is.
#[derive(Debug, Clone)]
pub struct SketchGroup {
    /// For each direction, the coordinate of each vector; zero for the
    /// vectors the group lacks.
    columns: Vec<[f32x4; 2]>,
    rests: [f32x4; 2],
}

impl SketchGroup {
    /// The group of `sketches`, at most [`SKETCH_GROUP`] of them, each the
    /// coordinates of a vector and its rest norm.
    ///
    /// # Panics
    ///
    /// With more than [`SKETCH_GROUP`] sketches, or coordinates of
    /// different lengths.
    pub fn new(sketches: &[(&[f32], f32)]) -> Self {
        assert!(sketches.len() <= SKETCH_GROUP, "too many sketches");
        let width = sketches
            .first()
            .map_or(0, |(coordinates, _)| coordinates.len());
        let lanes = |value: &dyn Fn(usize) -> f32| -> [f32x4; 2] {
            let values: [f32; SKETCH_GROUP] = std::array::from_fn(value);
            let [v0, v1, v2, v3, v4, v5, v6, v7] = values;
            [f32x4::from([v0, v1, v2, v3]), f32x4::from([v4, v5, v6, v7])]
        };
        for (coordinates, _) in sketches {
            assert_eq!(coordinates.len(), width, "sketches of different lengths");
        }
        Self {
            columns: (0..width)
                .map(|direction| {
                    lanes(&|vector| sketches.get(vector).map_or(0.0, |(c, _)| c[direction]))
                })
                .collect(),
            rests: lanes(&|vector| sketches.get(vector).map_or(0.0, |(_, rest)| *rest)),
        }
    }

    /// Bit `i` is clear only when the dot product of vector `i` with the
    /// row cannot reach `threshold`: its bound, from the row's coordinates
    /// and rest norm, is below it even with `margin` added for rounding.
    /// The bits of the vectors the group lacks mean nothing.
    ///
    /// # Panics
    ///
    /// When the row's coordinates are shorter than the group's.
    #[inline]
    pub fn reaching(&self, row: &[f32], row_rest: f32, threshold: f32, margin: f32) -> u8 {
        let row = &row[..self.columns.len()];
        // Two sums per half: the additions of one sum wait for each other.
        let mut sums = [f32x4::ZERO; 4];
        let (pairs, last) = self.columns.as_chunks::<2>();
        let (row_pairs, row_last) = row.as_chunks::<2>();
        for ([first, second], [a, b]) in pairs.iter().zip(row_pairs) {
            let (a, b) = (f32x4::splat(*a), f32x4::splat(*b));
            sums[0] += first[0] * a;
            sums[1] += first[1] * a;
            sums[2] += second[0] * b;
            sums[3] += second[1] * b;
        }
        for (column, value) in last.iter().zip(row_last) {
            let value = f32x4::splat(*value);
            sums[0] += column[0] * value;
            sums[1] += column[1] * value;
        }
        let (rest, slack, threshold) = (
            f32x4::splat(row_rest),
            f32x4::splat(margin),
            f32x4::splat(threshold),
        );
        let reaches =
            |sum: f32x4, rests: f32x4| (sum + rests * rest + slack).cmp_ge(threshold).move_mask();
        let low = reaches(sums[0] + sums[2], self.rests[0]);
        let high = reaches(sums[1] + sums[3], self.rests[1]);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let mask = (low | high << 4) as u8;
        mask
    }
}

/// The norm of `vector` after its first `prefix` lanes chunks, the part
/// [`dots_reaching`] bounds: the dimensions after the last whole chunk are
/// always in it.
pub fn rest_norm(vector: &[f32], prefix: usize) -> f32 {
    let rest = &vector[prefix.min(vector.len() / LANES) * LANES..];
    dot(rest, rest).sqrt()
}

/// The partial sums of [`dots`], accumulated chunk by chunk.
struct Blocked<'a, const N: usize> {
    queries: [&'a [f32]; N],
    query_chunks: [&'a [[f32; LANES]]; N],
    row: &'a [f32],
    row_chunks: &'a [[f32; LANES]],
    row_rest: &'a [f32],
    sums: [[f32x4; 2]; N],
    done: usize,
}

impl<'a, const N: usize> Blocked<'a, N> {
    fn new(queries: [&'a [f32]; N], row: &'a [f32]) -> Self {
        let (row_chunks, row_rest) = row.as_chunks::<LANES>();
        let query_chunks = queries.map(|query| {
            assert_eq!(query.len(), row.len(), "vectors of different lengths");
            &query.as_chunks::<LANES>().0[..row_chunks.len()]
        });
        Self {
            queries,
            query_chunks,
            row,
            row_chunks,
            row_rest,
            sums: [[f32x4::ZERO; 2]; N],
            done: 0,
        }
    }

    fn chunks(&self) -> usize {
        self.row_chunks.len()
    }

    /// Adds the products of chunks `done..end`, in order.
    fn accumulate(&mut self, end: usize) {
        for index in self.done..end {
            let (low, high) = halves(&self.row_chunks[index]);
            for (sum, query) in self.sums.iter_mut().zip(&self.query_chunks) {
                let (query_low, query_high) = halves(&query[index]);
                sum[0] += query_low * low;
                sum[1] += query_high * high;
            }
        }
        self.done = self.done.max(end);
    }

    fn totals(&self) -> [f32; N] {
        let mut totals = [0.0f32; N];
        let start = self.row.len() - self.row_rest.len();
        for ((total, [low, high]), query) in totals.iter_mut().zip(self.sums).zip(self.queries) {
            let (low, high) = (low.to_array(), high.to_array());
            *total = ((low[0] + high[0]) + (low[1] + high[1]))
                + ((low[2] + high[2]) + (low[3] + high[3]));
            for (a, b) in query[start..].iter().zip(self.row_rest) {
                *total += a * b;
            }
        }
        totals
    }
}

/// Lanes 0 to 3 and 4 to 7 of a chunk.
#[inline]
fn halves(chunk: &[f32; LANES]) -> (f32x4, f32x4) {
    let [l0, l1, l2, l3, l4, l5, l6, l7] = *chunk;
    (f32x4::from([l0, l1, l2, l3]), f32x4::from([l4, l5, l6, l7]))
}

/// Similarity of two unit vectors: their cosine, clamped to `[0, 1]`.
/// Opposite and unrelated texts are equally dissimilar; rounding can push
/// the cosine of identical texts slightly above 1.
pub fn semantic_similarity(left: &[f32], right: &[f32]) -> f32 {
    dot(left, right).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_matches_a_plain_sum_for_every_length() {
        for length in 0u8..40 {
            let left: Vec<f32> = (0..length).map(|i| (f32::from(i) * 0.37).sin()).collect();
            let right: Vec<f32> = (0..length).map(|i| (f32::from(i) * 0.11).cos()).collect();
            let plain: f64 = left
                .iter()
                .zip(&right)
                .map(|(a, b)| f64::from(*a) * f64::from(*b))
                .sum();
            assert!(
                (f64::from(dot(&left, &right)) - plain).abs() < 1e-5,
                "{length}"
            );
        }
    }

    /// A unit vector from a seed, possibly with its first `zeros` values 0.
    fn unit(length: usize, seed: u64, zeros: usize) -> Vec<f32> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut vector: Vec<f32> = (0..length)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                if i < zeros {
                    0.0
                } else {
                    f32::from(u16::try_from(state % 2001).unwrap()) / 1000.0 - 1.0
                }
            })
            .collect();
        let norm = dot(&vector, &vector).sqrt();
        if norm > 0.0 {
            for value in &mut vector {
                *value /= norm;
            }
        }
        vector
    }

    #[test]
    fn early_exit_never_drops_a_reachable_product() {
        let mut skipped = 0;
        for length in [8, 16, 63, 64, 128, 130, 256] {
            for seed in 0..300u64 {
                let queries: Vec<Vec<f32>> = (0..8)
                    .map(|i| unit(length, seed * 8 + i, (seed % 3) as usize * length / 3))
                    .collect();
                // Rows close to a query, far from all, or carried by the
                // dimensions after the prefix only.
                let row = match seed % 4 {
                    0 => {
                        let mut row = queries[(seed % 8) as usize].clone();
                        row[length / 2] += 0.05;
                        let norm = dot(&row, &row).sqrt();
                        for value in &mut row {
                            *value /= norm;
                        }
                        row
                    }
                    1 => unit(length, seed + 10_000, length / 2),
                    _ => unit(length, seed + 20_000, 0),
                };
                let queries: [&[f32]; 8] = std::array::from_fn(|i| queries[i].as_slice());
                let full = dots(queries, &row);
                for prefix in [0, 1, length / 16, length / 8 + 1] {
                    let rest = queries.map(|query| rest_norm(query, prefix));
                    let row_rest = rest_norm(&row, prefix);
                    for threshold in [0.1, 0.5, 0.8, 0.9, 0.99, 1.0] {
                        if let Some(found) =
                            dots_reaching(queries, rest, &row, row_rest, prefix, threshold)
                        {
                            assert_eq!(found.map(f32::to_bits), full.map(f32::to_bits));
                        } else {
                            skipped += 1;
                            assert!(full.iter().all(|&value| value < threshold));
                        }
                    }
                }
            }
        }
        assert!(skipped > 1000, "the bound must also skip: {skipped}");
    }

    #[test]
    fn a_sketch_group_bounds_each_of_its_vectors() {
        let (mut clear, mut set) = (0, 0);
        for width in [0, 1, 2, 5, 32, 33] {
            for count in 0..=8u8 {
                for seed in 0..40u8 {
                    let coordinates: Vec<Vec<f32>> = (0..count)
                        .map(|i| unit(width, u64::from(seed) * 16 + u64::from(i), 0))
                        .collect();
                    let rests: Vec<f32> = (0..count).map(|i| 0.1 * f32::from(i)).collect();
                    let sketches: Vec<(&[f32], f32)> = coordinates
                        .iter()
                        .zip(&rests)
                        .map(|(coordinates, rest)| (coordinates.as_slice(), *rest))
                        .collect();
                    let group = SketchGroup::new(&sketches);
                    // A row longer than the sketches: only their width is read.
                    let row = match seed % 3 {
                        0 if count > 0 => {
                            let mut row = coordinates[usize::from(seed % count)].clone();
                            row.push(9.0);
                            row
                        }
                        _ => unit(width + 1, u64::from(seed) + 5_000, 0),
                    };
                    let row_rest = 0.05 * f32::from(seed % 7);
                    for threshold in [0.2, 0.5, 0.8, 1.0] {
                        let reaching = group.reaching(&row, row_rest, threshold, 1e-5);
                        for (index, (coordinates, rest)) in sketches.iter().enumerate() {
                            let bound = dot(coordinates, &row[..width]) + rest * row_rest;
                            let bit = reaching >> index & 1 == 1;
                            // Sums in another order differ by rounding only.
                            if (bound - threshold).abs() > 1e-4 {
                                assert_eq!(bit, bound >= threshold, "{width} {count} {seed}");
                            }
                            clear += usize::from(!bit);
                            set += usize::from(bit);
                        }
                    }
                }
            }
        }
        assert!(clear > 1000 && set > 1000, "{clear} {set}");
    }

    #[test]
    #[should_panic(expected = "too many sketches")]
    fn a_sketch_group_refuses_a_ninth_vector() {
        let _ = SketchGroup::new(&[(&[][..], 0.0); 9]);
    }

    #[test]
    fn without_a_prefix_the_full_products_are_returned() {
        // Callers pass no norm then: zero norms must not skip anything.
        for length in [0, 4, 7, 8, 15, 16, 128] {
            let queries: Vec<Vec<f32>> = (0..8).map(|i| unit(length, i, 0)).collect();
            let queries: [&[f32]; 8] = std::array::from_fn(|i| queries[i].as_slice());
            let row = unit(length, 99, 0);
            let found = dots_reaching(queries, [0.0; 8], &row, 0.0, 0, 1.0);
            assert_eq!(
                found.map(|found| found.map(f32::to_bits)),
                Some(dots(queries, &row).map(f32::to_bits)),
                "{length}"
            );
        }
    }

    #[test]
    fn blocked_dots_equal_dot_bit_for_bit() {
        for length in (0u16..40).chain([255, 256, 257]) {
            let vector = |seed: f32| -> Vec<f32> {
                (0..length)
                    .map(|i| ((f32::from(i) + seed) * 0.731).sin() * 0.37)
                    .collect()
            };
            let (a, b, c, d, row) = (
                vector(1.0),
                vector(2.0),
                vector(3.0),
                vector(4.0),
                vector(5.0),
            );
            let expected = [&a, &b, &c, &d].map(|query| dot(query, &row).to_bits());
            assert_eq!(
                dots([&a, &b, &c, &d], &row).map(f32::to_bits),
                expected,
                "{length}"
            );
            assert_eq!(dots([&a], &row)[0].to_bits(), expected[0]);
            let eight = [&a, &b, &c, &d, &row, &d, &c, &a];
            let expected = eight.map(|query| dot(query, &row).to_bits());
            assert_eq!(
                dots(eight.map(Vec::as_slice), &row).map(f32::to_bits),
                expected
            );
            assert_eq!(dots::<0>([], &row), []);
        }
    }

    #[test]
    #[should_panic(expected = "different lengths")]
    fn blocked_dots_refuse_vectors_of_different_lengths() {
        dots([&[1.0, 2.0]], &[1.0]);
    }

    #[test]
    fn similarity_is_clamped_to_the_unit_interval() {
        let unit = [0.6, 0.8];
        assert_eq!(semantic_similarity(&unit, &[-0.6, -0.8]), 0.0);
        assert_eq!(semantic_similarity(&unit, &unit), 1.0);
        assert_eq!(semantic_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    }
}
