//! Int8 copies of embeddings, read before the `f32` rows to rule out the
//! rows that cannot reach a threshold.
//!
//! A row `r` is kept as codes `k` with a scale `s`, so that `r̂ = s·k` is
//! close to `r`, plus upper bounds on `‖r − r̂‖` and `‖r‖`. A query `q` is
//! quantized the same way. Then
//! `q·r = s_q·s_r·(k_q·k_r) + q̂·(r − r̂) + (q − q̂)·r`, and Cauchy–Schwarz
//! bounds the last two terms by `‖q̂‖·‖r − r̂‖` and `‖q − q̂‖·‖r‖`. The
//! integer dot product is exact, and a margin covers the rounding of the
//! `f32` dot product that scoring computes, so a row whose similarity
//! reaches the threshold is never ruled out: results are those of the full
//! scan, bit for bit.
//!
//! A scan of a large field is bound by memory bandwidth, and the codes are a
//! quarter of the row.

/// Largest code magnitude, symmetric so that negation is exact.
const MAX_CODE: f32 = 127.0;

/// Products summed in `i32` before widening: 2^16 products of at most 127²
/// stay below `i32::MAX`.
const EXACT_CHUNK: usize = 1 << 16;

/// What bounds the dot product of a query with one row, besides its codes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RowBound {
    scale: f32,
    /// Upper bound on the norm of the row minus its decoded codes.
    residual: f32,
    /// Upper bound on the norm of the row.
    norm: f32,
}

/// Appends the codes of `row` to `codes` and returns its bound.
pub(crate) fn quantize(row: &[f32], codes: &mut Vec<i8>) -> RowBound {
    let (scale, residual, norm) = encode(row, codes);
    RowBound {
        scale,
        residual: round_up(residual),
        norm: round_up(norm),
    }
}

/// A query quantized like the rows, to bound its dot product with each.
#[derive(Debug)]
pub(crate) struct QueryBound {
    codes: Vec<i8>,
    scale: f64,
    /// Weight of a row's residual: `‖q̂‖`.
    residual_weight: f64,
    /// Weight of a row's norm: `‖q − q̂‖`, plus the rounding margin.
    norm_weight: f64,
}

impl QueryBound {
    pub(crate) fn new(query: &[f32]) -> Self {
        let mut codes = Vec::with_capacity(query.len());
        let (scale, residual, norm) = encode(query, &mut codes);
        let decoded = codes
            .iter()
            .map(|&code| (f64::from(scale) * f64::from(code)).powi(2))
            .sum::<f64>()
            .sqrt();
        // `dot` sums `n` products in `f32`, within `n·ε·‖q‖·‖r‖` of the
        // exact value. The margin dwarfs the `f64` rounding of the bound.
        #[allow(clippy::cast_precision_loss)]
        let rounding = query.len() as f64 * f64::from(f32::EPSILON) * norm;
        Self {
            codes,
            scale: f64::from(scale),
            residual_weight: decoded,
            norm_weight: residual + rounding,
        }
    }

    /// An upper bound on `univo_core::dot(query, row)`, from the codes and
    /// bound of `row`.
    pub(crate) fn upper(&self, codes: &[i8], row: RowBound) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let exact = if codes.len() <= EXACT_CHUNK {
            f64::from(dot_chunk(&self.codes, codes))
        } else {
            dot_codes(&self.codes, codes) as f64
        };
        self.scale * f64::from(row.scale) * exact
            + self.residual_weight * f64::from(row.residual)
            + self.norm_weight * f64::from(row.norm)
    }
}

/// The codes of `vector`, appended to `codes`, with its scale and the norms
/// of its residual and of itself.
fn encode(vector: &[f32], codes: &mut Vec<i8>) -> (f32, f64, f64) {
    let max = vector
        .iter()
        .fold(0.0f32, |max, value| max.max(value.abs()));
    let scale = max / MAX_CODE;
    // All zeros, or so small that the scale underflows: every code is 0 and
    // the residual is the whole vector.
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let (mut residual, mut norm) = (0.0f64, 0.0f64);
    codes.extend(vector.iter().map(|&value| {
        #[allow(clippy::cast_possible_truncation)]
        let code = (value / scale).round().clamp(-MAX_CODE, MAX_CODE) as i8;
        let error = f64::from(value) - f64::from(scale) * f64::from(code);
        residual += error * error;
        norm += f64::from(value) * f64::from(value);
        code
    }));
    (scale, residual.sqrt(), norm.sqrt())
}

/// The smallest `f32` at least `value`.
fn round_up(value: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let rounded = value as f32;
    if f64::from(rounded) < value {
        rounded.next_up()
    } else {
        rounded
    }
}

/// Exact dot product of two code vectors of the same length. Integer sums
/// are associative, so the compiler vectorizes the loop freely.
fn dot_codes(left: &[i8], right: &[i8]) -> i64 {
    debug_assert_eq!(left.len(), right.len());
    left.chunks(EXACT_CHUNK)
        .zip(right.chunks(EXACT_CHUNK))
        .map(|(left, right)| i64::from(dot_chunk(left, right)))
        .sum()
}

/// Dot product of at most [`EXACT_CHUNK`] codes.
#[inline]
fn dot_chunk(left: &[i8], right: &[i8]) -> i32 {
    left.iter()
        .zip(right)
        .map(|(&a, &b)| i32::from(a) * i32::from(b))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use univo_core::dot;

    /// Pseudo-random vectors of `dimension`, unit or scaled.
    fn vectors(count: usize, dimension: usize, seed: u64) -> Vec<Vec<f32>> {
        let mut state = seed;
        (0..count)
            .map(|_| {
                let row: Vec<f32> = (0..dimension)
                    .map(|_| {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        f32::from(u16::try_from(state % 2001).unwrap()) / 1000.0 - 1.0
                    })
                    .collect();
                let norm = row.iter().map(|v| v * v).sum::<f32>().sqrt();
                row.iter().map(|v| v / norm).collect()
            })
            .collect()
    }

    #[test]
    fn the_bound_is_never_below_the_dot_product() {
        for dimension in [1, 2, 7, 8, 128, 257] {
            let rows = vectors(200, dimension, 11 + dimension as u64);
            // Near-duplicates, opposites and scaled rows stress the bound
            // where it is tightest.
            let mut queries = vectors(20, dimension, 3);
            queries.push(rows[0].clone());
            queries.push(rows[1].iter().map(|v| -v).collect());
            queries.push(rows[2].iter().map(|v| v * 1.000_5).collect());
            for query in &queries {
                let bound = QueryBound::new(query);
                for row in &rows {
                    let mut codes = Vec::new();
                    let row_bound = quantize(row, &mut codes);
                    let upper = bound.upper(&codes, row_bound);
                    assert!(
                        f64::from(dot(query, row)) <= upper,
                        "{dimension}: {} > {upper}",
                        dot(query, row)
                    );
                }
            }
        }
    }

    #[test]
    fn the_bound_is_tight_for_unit_vectors() {
        let rows = vectors(100, 128, 5);
        let query = &rows[0];
        let bound = QueryBound::new(query);
        for row in &rows {
            let mut codes = Vec::new();
            let row_bound = quantize(row, &mut codes);
            let gap = bound.upper(&codes, row_bound) - f64::from(dot(query, row));
            assert!(gap < 0.02, "{gap}");
        }
    }

    #[test]
    fn zero_and_tiny_vectors_keep_a_valid_bound() {
        for row in [
            vec![0.0f32; 16],
            vec![f32::MIN_POSITIVE / 4.0; 16],
            vec![-1e-30; 16],
        ] {
            let mut codes = Vec::new();
            let row_bound = quantize(&row, &mut codes);
            assert_eq!(codes.len(), 16);
            for query in [vec![0.25f32; 16], vec![-0.25f32; 16]] {
                let upper = QueryBound::new(&query).upper(&codes, row_bound);
                assert!(f64::from(dot(&query, &row)) <= upper);
            }
        }
    }

    #[test]
    fn codes_beyond_one_chunk_are_summed_exactly() {
        let left = vec![-127i8; EXACT_CHUNK * 2 + 3];
        let right = vec![-127i8; EXACT_CHUNK * 2 + 3];
        let expected = 127i64 * 127 * i64::try_from(left.len()).unwrap();
        assert_eq!(dot_codes(&left, &right), expected);
    }
}
