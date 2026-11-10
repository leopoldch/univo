//! Native static inference: `Model2Vec`'s mean of token vectors, computed
//! from the stored table without expanding it, bit for bit as
//! `model2vec-rs` 0.3.0 computes it.
//!
//! An int8 table stays in the bytes of `model.safetensors`, a quarter of
//! the `f32` copy `model2vec-rs` builds. Token rows are summed in the same
//! order with the same operations; for int8 rows without weights the sum
//! is taken in `i32`, which equals the `f32` sum exactly while it stays
//! below 2^24.

mod safetensors;
mod tokenizer;
mod vocab;

use rayon::prelude::*;

use safetensors::{Dtype, f16_to_f32};
use tokenizer::Scratch;
pub(crate) use tokenizer::Tokenizer;

#[derive(Debug)]
enum Table {
    /// The file's bytes and the offset of the int8 rows in them.
    I8 {
        bytes: Vec<u8>,
        offset: usize,
    },
    F32(Vec<f32>),
}

#[derive(Debug)]
pub(crate) struct NativeModel {
    tokenizer: Tokenizer,
    table: Table,
    dimension: usize,
    /// Row of each token id, when the model remaps them.
    mapping: Option<Vec<usize>>,
    /// Scale of each token id, when the model weights them.
    weights: Option<Vec<f32>>,
}

/// Texts per parallel task, and the smallest batch worth splitting.
const TASK_TEXTS: usize = 256;

/// Tokens whose int8 sum is exact in `f32` at every step: an int8 value
/// can be -128, and 128 · n ≤ 2^24 keeps every partial sum representable.
const EXACT_INT8_TOKENS: usize = (1 << 24) / 128;

impl NativeModel {
    /// The model in `weights`, tokenized by `tokenizer`.
    ///
    /// # Errors
    ///
    /// Returns a description when the file is not a static model, or a
    /// token has no row.
    pub(crate) fn new(tokenizer: Tokenizer, weights: Vec<u8>) -> Result<Self, String> {
        let tensors = safetensors::parse(&weights)?;
        let embeddings = ["embeddings", "0", "embedding.weight"]
            .iter()
            .find_map(|name| tensors.get(*name))
            .ok_or("embeddings tensor not found")?;
        let &[rows, dimension] = embeddings.shape.as_slice() else {
            return Err("embedding tensor is not 2-D".to_owned());
        };
        if dimension == 0 {
            return Err("the model has no dimension".to_owned());
        }
        let raw = &weights[embeddings.start..embeddings.end];
        let table = match embeddings.dtype {
            Dtype::I8 => None,
            Dtype::F32 => Some(decode(raw, f32::from_le_bytes)),
            Dtype::F16 => Some(decode(raw, |b| f16_to_f32(u16::from_le_bytes(b)))),
            other => return Err(format!("unsupported tensor dtype: {other:?}")),
        };
        let scales = match tensors.get("weights") {
            None => None,
            Some(tensor) => {
                let raw = &weights[tensor.start..tensor.end];
                Some(match tensor.dtype {
                    #[allow(clippy::cast_possible_truncation)]
                    Dtype::F64 => decode(raw, |b| f64::from_le_bytes(b) as f32),
                    Dtype::F32 => decode(raw, f32::from_le_bytes),
                    Dtype::F16 => decode(raw, |b| f16_to_f32(u16::from_le_bytes(b))),
                    other => return Err(format!("unsupported weights dtype: {other:?}")),
                })
            }
        };
        let mapping = match tensors.get("mapping") {
            None => None,
            Some(tensor) => {
                let raw = &weights[tensor.start..tensor.end];
                // As `model2vec-rs`: a negative index wraps to a huge row,
                // refused below.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Some(match tensor.dtype {
                    Dtype::I64 => decode(raw, |b| i64::from_le_bytes(b) as usize),
                    Dtype::I32 => decode(raw, |b| i32::from_le_bytes(b) as usize),
                    other => return Err(format!("unsupported mapping dtype: {other:?}")),
                })
            }
        };
        let offset = embeddings.start;
        let table = match table {
            Some(values) => Table::F32(values),
            None => Table::I8 {
                bytes: weights,
                offset,
            },
        };
        let model = Self {
            tokenizer,
            table,
            dimension,
            mapping,
            weights: scales,
        };
        // `model2vec-rs` would panic on the first text with such a token.
        if model.tokenizer.ids().any(|id| model.row(id) >= rows) {
            return Err("a token has no embedding row".to_owned());
        }
        Ok(model)
    }

    pub(crate) fn dimension(&self) -> usize {
        self.dimension
    }

    fn row(&self, id: u32) -> usize {
        let id = id as usize;
        self.mapping
            .as_ref()
            .and_then(|mapping| mapping.get(id))
            .copied()
            .unwrap_or(id)
    }

    /// One row of `dimension` values per text in `out`, zeros for a text
    /// without known tokens. Runs on the current Rayon pool when the batch
    /// is large enough to split.
    pub(crate) fn encode_into(&self, texts: &[&str], out: &mut [f32]) {
        let task = TASK_TEXTS * self.dimension;
        if texts.len() < 2 * TASK_TEXTS {
            self.encode_sequential(texts, out);
        } else {
            out.par_chunks_mut(task)
                .zip(texts.par_chunks(TASK_TEXTS))
                .for_each(|(out, texts)| self.encode_sequential(texts, out));
        }
    }

    fn encode_sequential(&self, texts: &[&str], out: &mut [f32]) {
        let mut scratch = Scratch::default();
        let mut ids = Vec::new();
        let mut sums = vec![0i32; self.dimension];
        for (text, row) in texts.iter().zip(out.chunks_exact_mut(self.dimension)) {
            ids.clear();
            self.tokenizer.tokenize(text, &mut scratch, &mut ids);
            let unk = self.tokenizer.unk();
            ids.retain(|&id| id != unk);
            self.pool(&ids, row, &mut sums);
        }
    }

    /// `model2vec-rs`'s `pool_ids`, operation for operation.
    fn pool(&self, ids: &[u32], row: &mut [f32], sums: &mut [i32]) {
        row.fill(0.0);
        match &self.table {
            Table::I8 { bytes, offset }
                if self.weights.is_none() && ids.len() <= EXACT_INT8_TOKENS =>
            {
                sums.fill(0);
                for &id in ids {
                    let start = offset + self.row(id) * self.dimension;
                    for (sum, &byte) in sums.iter_mut().zip(&bytes[start..start + self.dimension]) {
                        *sum += i32::from(byte.cast_signed());
                    }
                }
                for (value, &sum) in row.iter_mut().zip(sums.iter()) {
                    #[allow(clippy::cast_precision_loss)]
                    {
                        *value = sum as f32;
                    }
                }
            }
            Table::I8 { bytes, offset } => {
                for &id in ids {
                    let start = offset + self.row(id) * self.dimension;
                    let scale = self.scale(id);
                    for (sum, &byte) in row.iter_mut().zip(&bytes[start..start + self.dimension]) {
                        *sum += f32::from(byte.cast_signed()) * scale;
                    }
                }
            }
            Table::F32(values) => {
                for &id in ids {
                    let start = self.row(id) * self.dimension;
                    let scale = self.scale(id);
                    for (sum, &value) in row.iter_mut().zip(&values[start..start + self.dimension])
                    {
                        *sum += value * scale;
                    }
                }
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let denominator = ids.len().max(1) as f32;
        for value in row.iter_mut() {
            *value /= denominator;
        }
        let norm = row.iter().map(|&v| v * v).sum::<f32>().sqrt().max(1e-12);
        for value in row.iter_mut() {
            *value /= norm;
        }
    }

    fn scale(&self, id: u32) -> f32 {
        self.weights
            .as_ref()
            .and_then(|weights| weights.get(id as usize))
            .copied()
            .unwrap_or(1.0)
    }
}

/// Little-endian values of `N` bytes each.
fn decode<const N: usize, T>(raw: &[u8], value: impl Fn([u8; N]) -> T) -> Vec<T> {
    raw.as_chunks::<N>()
        .0
        .iter()
        .map(|&bytes| value(bytes))
        .collect()
}
