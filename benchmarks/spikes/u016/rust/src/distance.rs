use fast_hnsw::distance::Distance;

pub struct Euclidean;
impl Distance for Euclidean {
    fn distance(&self, a: &[f32], b: &[f32]) -> f32 {
        let mut sums = [0.0; 8];
        let (chunks_a, rest_a) = a.as_chunks::<8>();
        let (chunks_b, rest_b) = b.as_chunks::<8>();
        for (a, b) in chunks_a.iter().zip(chunks_b) {
            for lane in 0..8 {
                let difference = a[lane] - b[lane];
                sums[lane] += difference * difference;
            }
        }
        let mut sum = sums.iter().sum::<f32>();
        for (a, b) in rest_a.iter().zip(rest_b) {
            sum += (a - b) * (a - b);
        }
        sum.sqrt()
    }
}
