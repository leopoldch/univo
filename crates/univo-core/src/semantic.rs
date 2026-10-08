//! Semantic similarity of embeddings, as defined in `docs/scoring.md`.

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
    fn similarity_is_clamped_to_the_unit_interval() {
        let unit = [0.6, 0.8];
        assert_eq!(semantic_similarity(&unit, &[-0.6, -0.8]), 0.0);
        assert_eq!(semantic_similarity(&unit, &unit), 1.0);
        assert_eq!(semantic_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    }
}
