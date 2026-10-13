//! Semantic similarity of embeddings, as defined in `docs/scoring.md`.

use wide::f32x4;

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
    let (row_chunks, row_rest) = row.as_chunks::<LANES>();
    let chunks = queries.map(|query| {
        assert_eq!(query.len(), row.len(), "vectors of different lengths");
        &query.as_chunks::<LANES>().0[..row_chunks.len()]
    });
    let mut sums = [[f32x4::ZERO; 2]; N];
    for (index, chunk) in row_chunks.iter().enumerate() {
        let (low, high) = halves(chunk);
        for (sum, query) in sums.iter_mut().zip(&chunks) {
            let (query_low, query_high) = halves(&query[index]);
            sum[0] += query_low * low;
            sum[1] += query_high * high;
        }
    }
    let mut totals = [0.0f32; N];
    for ((total, [low, high]), query) in totals.iter_mut().zip(sums).zip(queries) {
        let (low, high) = (low.to_array(), high.to_array());
        *total =
            ((low[0] + high[0]) + (low[1] + high[1])) + ((low[2] + high[2]) + (low[3] + high[3]));
        for (a, b) in query[row.len() - row_rest.len()..].iter().zip(row_rest) {
            *total += a * b;
        }
    }
    totals
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
