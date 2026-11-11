//! Sketches of one field's embeddings, for the dense lane of all-pairs
//! reports: each row's coordinates on a few orthonormal directions and the
//! norm of what they leave out (`univo_core::SketchGroup`).
//!
//! The dot product of two rows is at most the dot product of their
//! coordinates plus the product of their rest norms, whatever the
//! directions. The directions are fitted to the field, as the principal
//! directions of a sample of its rows, so that most of each row lies along
//! them: a few coordinates then rule out most pairs, and the scan reads a
//! quarter of each row or less. A pair the bound admits is compared on the
//! rows themselves, so reports do not depend on the sketch.
//!
//! The directions are computed and orthonormalized in `f64`: their Gram
//! matrix is the identity within about `1e-15`. A coordinate is rounded to
//! `f32` once, and a rest norm is rounded up. For unit rows, the bound
//! computed from them exceeds the exact one by less than the float sum of
//! `SKETCH_WIDTH` products allows, which `univo_core::rounding_margin`
//! covers together with the rounding of the final dot product.

use rayon::prelude::*;

use crate::vectors::FieldVectors;

/// Directions of a sketch: 128 bytes per row.
pub(crate) const SKETCH_WIDTH: usize = 32;

/// Pairs of an all-pairs report per query from which sketching a field
/// pays for fitting the directions and projecting every row: a partition
/// of about 8,000 records. The sketch breaks even near 6,000.
pub(crate) const SKETCH_PAIRS_PER_ROW: usize = 4096;

/// Rows the directions are fitted to, evenly spread over the field.
const SAMPLE: usize = 1024;

/// Rounds of orthogonal iteration: the directions need not have converged,
/// only to be orthonormal.
const ROUNDS: usize = 8;

/// The sketch of each row of a field.
#[derive(Debug)]
pub(crate) struct DenseSketch {
    coordinates: Vec<f32>,
    rests: Vec<f32>,
}

impl DenseSketch {
    /// The sketches of `vectors`, or `None` when its rows are too short or
    /// span too few directions.
    pub(crate) fn new(vectors: &FieldVectors) -> Option<Self> {
        let dimension = vectors.dimension;
        if dimension < 2 * SKETCH_WIDTH {
            return None;
        }
        let rows = vectors.data.len() / dimension;
        let directions = directions(&vectors.data, dimension)?;
        let mut coordinates = vec![0.0f32; rows * SKETCH_WIDTH];
        let mut rests = vec![0.0f32; rows];
        let project = |((row, coordinates), rest): ((&[f32], &mut [f32]), &mut f32)| {
            let mut kept = 0.0;
            for (direction, coordinate) in directions.chunks_exact(dimension).zip(coordinates) {
                let value = dot(direction, row);
                kept += value * value;
                #[allow(clippy::cast_possible_truncation)]
                let rounded = value as f32;
                *coordinate = rounded;
            }
            let norm: f64 = row.iter().map(|&value| f64::from(value).powi(2)).sum();
            // Rounded up, past the rounding of the subtraction and the cast.
            #[allow(clippy::cast_possible_truncation)]
            let above = ((norm - kept).max(0.0).sqrt() * (1.0 + 1e-6) + 1e-6) as f32;
            *rest = above;
        };
        if rayon::current_thread_index().is_some() {
            vectors
                .data
                .par_chunks_exact(dimension)
                .zip(coordinates.par_chunks_exact_mut(SKETCH_WIDTH))
                .zip(rests.par_iter_mut())
                .for_each(project);
        } else {
            vectors
                .data
                .chunks_exact(dimension)
                .zip(coordinates.as_chunks_mut::<SKETCH_WIDTH>().0)
                .map(|(row, coordinates)| (row, coordinates.as_mut_slice()))
                .zip(rests.iter_mut())
                .for_each(project);
        }
        Some(Self { coordinates, rests })
    }

    /// The coordinates and the rest norm of a row.
    #[inline]
    pub(crate) fn row(&self, row: usize) -> (&[f32], f32) {
        (
            &self.coordinates[row * SKETCH_WIDTH..(row + 1) * SKETCH_WIDTH],
            self.rests[row],
        )
    }
}

/// `SKETCH_WIDTH` orthonormal directions of `dimension` values each, close
/// to the principal directions of the rows of `data`: those along which a
/// sample of rows has the largest second moments. `None` when the sample
/// spans fewer directions.
fn directions(data: &[f32], dimension: usize) -> Option<Vec<f64>> {
    let rows = data.len() / dimension;
    let mut moments = vec![0.0f64; dimension * dimension];
    for row in data
        .chunks_exact(dimension)
        .step_by(rows.div_ceil(SAMPLE).max(1))
    {
        for (line, &value) in moments.chunks_exact_mut(dimension).zip(row) {
            let value = f64::from(value);
            for (moment, &other) in line.iter_mut().zip(row) {
                *moment += value * f64::from(other);
            }
        }
    }
    // Orthogonal iteration from fixed pseudo-random directions.
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut directions: Vec<f64> = (0..SKETCH_WIDTH * dimension)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            f64::from(u32::try_from(state >> 40).unwrap_or(0)) / f64::from(1u32 << 23) - 1.0
        })
        .collect();
    orthonormalize(&mut directions, dimension)?;
    let mut next = vec![0.0f64; directions.len()];
    for _ in 0..ROUNDS {
        for (direction, image) in directions
            .chunks_exact(dimension)
            .zip(next.chunks_exact_mut(dimension))
        {
            for (value, line) in image.iter_mut().zip(moments.chunks_exact(dimension)) {
                *value = line.iter().zip(direction).map(|(a, b)| a * b).sum();
            }
        }
        std::mem::swap(&mut directions, &mut next);
        orthonormalize(&mut directions, dimension)?;
    }
    Some(directions)
}

/// Makes the directions orthonormal, each against the previous ones
/// (modified Gram–Schmidt, twice: once leaves an error that grows with the
/// angle removed). `None` when one of them depends on the previous ones.
fn orthonormalize(directions: &mut [f64], dimension: usize) -> Option<()> {
    for index in 0..directions.len() / dimension {
        let (previous, rest) = directions.split_at_mut(index * dimension);
        let direction = &mut rest[..dimension];
        let before: f64 = direction.iter().map(|value| value * value).sum();
        for _ in 0..2 {
            for other in previous.chunks_exact(dimension) {
                let along: f64 = direction.iter().zip(other).map(|(a, b)| a * b).sum();
                for (value, other) in direction.iter_mut().zip(other) {
                    *value -= along * other;
                }
            }
        }
        let norm = direction
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt();
        if !(norm.is_finite() && norm > 1e-9 * before.sqrt() && norm > 0.0) {
            return None;
        }
        for value in direction.iter_mut() {
            *value /= norm;
        }
    }
    Some(())
}

/// Dot product of a direction and a row, in `f64`, on four sums.
fn dot(direction: &[f64], row: &[f32]) -> f64 {
    let (chunks, rest) = direction.as_chunks::<4>();
    let (row_chunks, row_rest) = row.as_chunks::<4>();
    let mut sums = [0.0f64; 4];
    for (a, b) in chunks.iter().zip(row_chunks) {
        for lane in 0..4 {
            sums[lane] += a[lane] * f64::from(b[lane]);
        }
    }
    let mut total = (sums[0] + sums[2]) + (sums[1] + sums[3]);
    for (a, b) in rest.iter().zip(row_rest) {
        total += a * f64::from(*b);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit rows around `centers` shared directions, from a seed.
    fn rows(count: usize, dimension: usize, centers: usize, seed: u64) -> Vec<f32> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            f32::from(u16::try_from(state % 2001).unwrap()) / 1000.0 - 1.0
        };
        let bases: Vec<Vec<f32>> = (0..centers)
            .map(|_| (0..dimension).map(|_| next()).collect())
            .collect();
        let mut data = Vec::with_capacity(count * dimension);
        for row in 0..count {
            let base = &bases[row % centers];
            let vector: Vec<f32> = base.iter().map(|value| value + 0.3 * next()).collect();
            let norm = univo_core::dot(&vector, &vector).sqrt();
            data.extend(vector.iter().map(|value| value / norm));
        }
        data
    }

    fn field(data: Vec<f32>, dimension: usize) -> FieldVectors {
        let rows = (0..data.len() / dimension)
            .map(|row| Some(u32::try_from(row).unwrap()))
            .collect();
        FieldVectors::from_rows(dimension, data, rows)
    }

    #[test]
    fn directions_are_orthonormal() {
        let dimension = 96;
        let data = rows(500, dimension, 40, 1);
        let directions = directions(&data, dimension).unwrap();
        for (i, a) in directions.chunks_exact(dimension).enumerate() {
            for (j, b) in directions.chunks_exact(dimension).enumerate() {
                let product: f64 = a.iter().zip(b).map(|(a, b)| a * b).sum();
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((product - expected).abs() < 1e-12, "{i} {j}: {product}");
            }
        }
    }

    #[test]
    fn the_bound_never_falls_below_the_dot_product() {
        for (dimension, centers, seed) in [(64, 3, 1), (128, 50, 2), (130, 200, 3), (256, 40, 4)] {
            let vectors = field(rows(400, dimension, centers, seed), dimension);
            let Some(sketch) = DenseSketch::new(&vectors) else {
                // Three centers span too few directions.
                assert!(centers < SKETCH_WIDTH, "{dimension}");
                continue;
            };
            let margin = univo_core::rounding_margin(dimension);
            let (mut tight, mut pairs) = (0, 0);
            for a in 0..400 {
                for b in a..400 {
                    let exact = univo_core::dot(vectors.get(a).unwrap(), vectors.get(b).unwrap());
                    let ((left, left_rest), (right, right_rest)) = (sketch.row(a), sketch.row(b));
                    let bound = univo_core::dot(left, right) + left_rest * right_rest;
                    assert!(
                        bound + margin >= exact,
                        "{dimension} {a} {b}: {bound} {exact}"
                    );
                    pairs += 1;
                    tight += usize::from(bound - exact < 0.2);
                }
            }
            // With about as many centers as directions, the directions
            // follow the data: the bound is not vacuous.
            assert!(
                centers > 50 || tight * 2 > pairs,
                "{dimension}: {tight} of {pairs}"
            );
        }
    }

    #[test]
    fn short_or_degenerate_fields_have_no_sketch() {
        assert!(DenseSketch::new(&field(rows(100, 32, 10, 1), 32)).is_none());
        assert!(DenseSketch::new(&field(vec![0.125; 64 * 100], 64)).is_none());
        assert!(DenseSketch::new(&field(Vec::new(), 128)).is_none());
    }
}
