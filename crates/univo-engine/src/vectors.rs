//! Embeddings of the semantic text fields of prepared records, one
//! contiguous buffer per field. Hybrid fields are scored lexically and not
//! embedded.

use std::collections::HashMap;

use crate::EmbeddingCache;
use crate::quantized::{QueryBound, RowBound, quantize};
use univo_core::{
    Error, FieldKind, FieldState, PreparedRecord, Result, Schema, TextMode, semantic_similarity,
};
use univo_embed::{Embedder, ModelDescriptor};

/// Unit vectors of one field, by record in insertion order. Records with the
/// same text share one row; a record without an embedding, because its text
/// is missing or has no token the model knows, has none.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldVectors {
    pub(crate) dimension: usize,
    pub(crate) data: Vec<f32>,
    /// Int8 codes of `data`, row by row, and their bounds, derived from
    /// `data` and never stored: see [`crate::quantized`].
    codes: Vec<i8>,
    bounds: Vec<RowBound>,
    /// Row of each record in `data`, or `None`.
    pub(crate) rows: Vec<Option<u32>>,
}

impl FieldVectors {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension,
            data: Vec::new(),
            codes: Vec::new(),
            bounds: Vec::new(),
            rows: Vec::new(),
        }
    }

    /// Embeddings stored row after row in `data`, with the row of each
    /// record.
    pub(crate) fn from_rows(dimension: usize, data: Vec<f32>, rows: Vec<Option<u32>>) -> Self {
        let mut codes = Vec::with_capacity(data.len());
        let bounds = data
            .chunks_exact(dimension.max(1))
            .map(|row| quantize(row, &mut codes))
            .collect();
        Self {
            dimension,
            data,
            codes,
            bounds,
            rows,
        }
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Number of records, with or without an embedding.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The embedding of the record at `slot`, if it has one.
    pub fn get(&self, slot: usize) -> Option<&[f32]> {
        let row = self.rows.get(slot).copied().flatten()? as usize;
        Some(&self.data[row * self.dimension..(row + 1) * self.dimension])
    }

    /// The similarity of `query` to the record at `slot`, as
    /// [`semantic_similarity`] computes it, or `None` when the record has no
    /// embedding or its codes show that the similarity is below
    /// `threshold`. `bound` is [`QueryBound::new`] of `query`, which has the
    /// field's dimension.
    #[inline]
    pub(crate) fn similarity_reaching(
        &self,
        query: &[f32],
        bound: &QueryBound,
        slot: usize,
        threshold: f32,
    ) -> Option<f32> {
        let row = self.rows.get(slot).copied().flatten()? as usize;
        let span = row * self.dimension..(row + 1) * self.dimension;
        // A similarity is clamped at 0: only a positive threshold can rule
        // out a row with a negative dot product.
        if threshold > 0.0
            && bound.upper(&self.codes[span.clone()], self.bounds[row]) < f64::from(threshold)
        {
            return None;
        }
        Some(semantic_similarity(query, &self.data[span]))
    }

    /// Bytes held by the embeddings, their codes and their row table.
    pub fn heap_bytes(&self) -> usize {
        self.data.capacity() * size_of::<f32>()
            + self.codes.capacity()
            + self.bounds.capacity() * size_of::<RowBound>()
            + self.rows.capacity() * size_of::<Option<u32>>()
    }

    /// Row remapping for active slots, without copying vector data.
    pub(crate) fn selected_rows(
        &self,
        slots: impl Iterator<Item = usize>,
    ) -> Result<(Vec<Option<u32>>, Vec<u32>)> {
        let mut mapping = HashMap::new();
        let mut source = Vec::new();
        let mut selected = Vec::new();
        for slot in slots {
            let row = match self
                .rows
                .get(slot)
                .copied()
                .ok_or_else(Error::schema_mismatch)?
            {
                Some(row) => {
                    let next = if let Some(&next) = mapping.get(&row) {
                        next
                    } else {
                        let next = u32::try_from(source.len())
                            .map_err(|_| Error::index_capacity(source.len(), MAX_ROWS))?;
                        mapping.insert(row, next);
                        source.push(row);
                        next
                    };
                    Some(next)
                }
                None => None,
            };
            selected.push(row);
        }
        Ok((selected, source))
    }

    /// Stores a new row and returns its number.
    fn add_row(&mut self, vector: &[f32]) -> Result<u32> {
        debug_assert_eq!(vector.len(), self.dimension);
        let row = self.data.len() / self.dimension.max(1);
        let row = u32::try_from(row).map_err(|_| Error::index_capacity(row + 1, MAX_ROWS))?;
        self.data.extend_from_slice(vector);
        self.bounds.push(quantize(vector, &mut self.codes));
        Ok(row)
    }
}

/// Distinct embeddings one field can hold.
const MAX_ROWS: usize = u32::MAX as usize;
/// About 1 MiB of transient float data, with at most 1,024 texts per call.
const MAX_BATCH_VALUES: usize = 256 * 1024;

/// Embeddings of every embedded field of a set of records, indexed by the
/// schema's field order, with the schema and the model that produced them.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordVectors {
    pub(crate) schema: u64,
    pub(crate) model: ModelDescriptor,
    pub(crate) fields: Vec<Option<FieldVectors>>,
}

impl RecordVectors {
    /// Whether these embeddings were computed for `records` records of the
    /// schema with this preparation fingerprint: same field layout, one
    /// entry per record in every embedded field.
    pub(crate) fn covers(&self, schema: u64, records: usize) -> bool {
        self.schema == schema
            && self
                .fields
                .iter()
                .flatten()
                .all(|field| field.len() == records)
    }

    pub(crate) fn empty(schema: &Schema, model: ModelDescriptor) -> Self {
        let fields = (0..schema.fields().len())
            .map(|i| is_embedded(schema, i).then(|| FieldVectors::new(model.dimension)))
            .collect();
        Self {
            schema: schema.preparation_fingerprint(),
            model,
            fields,
        }
    }

    /// Compact selected slots without re-encoding, preserving shared rows.
    pub(crate) fn select(&self, slots: &[usize]) -> Result<Self> {
        let mut selected = Self {
            schema: self.schema,
            model: self.model.clone(),
            fields: Vec::new(),
        };
        for field in &self.fields {
            let Some(field) = field else {
                selected.fields.push(None);
                continue;
            };
            let (rows, source) = field.selected_rows(slots.iter().copied())?;
            let mut output = FieldVectors::new(field.dimension);
            for row in source {
                let start = row as usize * field.dimension;
                output.add_row(&field.data[start..start + field.dimension])?;
            }
            output.rows = rows;
            selected.fields.push(Some(output));
        }
        Ok(selected)
    }

    pub(crate) fn reuse(
        &mut self,
        old: &Self,
        slots: &[Option<usize>],
        records: &[PreparedRecord],
        store: &crate::RecordStore,
    ) -> Result<()> {
        for (index, field) in self.fields.iter_mut().enumerate() {
            let (Some(field), Some(previous)) = (field, old.field(index)) else {
                continue;
            };
            let mut reused = HashMap::new();
            for (position, slot) in slots.iter().enumerate() {
                let Some(slot) = slot else {
                    continue;
                };
                if records[position].fields()[index].state
                    != store.records()[*slot].fields()[index].state
                {
                    continue;
                }
                field.rows[position] = match previous.rows[*slot] {
                    Some(row) => {
                        let next = if let Some(&next) = reused.get(&row) {
                            next
                        } else {
                            let next = field
                                .add_row(previous.get(*slot).ok_or_else(Error::schema_mismatch)?)?;
                            reused.insert(row, next);
                            next
                        };
                        Some(next)
                    }
                    None => None,
                };
            }
        }
        Ok(())
    }

    /// The model that produced these embeddings.
    pub fn model(&self) -> &ModelDescriptor {
        &self.model
    }

    /// The embeddings of field `index`, when the schema embeds it.
    pub fn field(&self, index: usize) -> Option<&FieldVectors> {
        self.fields.get(index)?.as_ref()
    }

    /// The embedding of each field of the record at `slot`, in schema order,
    /// as [`univo_core::compare_with_vectors`] takes them.
    pub fn record(&self, slot: usize) -> Vec<Option<&[f32]>> {
        let mut vectors = Vec::new();
        self.record_into(slot, &mut vectors);
        vectors
    }

    /// [`Self::record`] into `vectors`, replacing its content, so that
    /// scoring many candidates reuses one buffer.
    pub fn record_into<'a>(&'a self, slot: usize, vectors: &mut Vec<Option<&'a [f32]>>) {
        vectors.clear();
        vectors.extend(
            self.fields
                .iter()
                .map(|field| field.as_ref().and_then(|field| field.get(slot))),
        );
    }

    /// Checks that [`RecordVectors::extend`] would accept `other`, without
    /// changing anything.
    ///
    /// # Errors
    ///
    /// As [`RecordVectors::extend`].
    pub fn check_extend(&self, other: &Self) -> Result<()> {
        self.offsets(other).map(drop)
    }

    /// Appends the embeddings of later records, embedded with the same
    /// schema and model.
    ///
    /// # Errors
    ///
    /// Leaves `self` unchanged and returns `state.schema_mismatch` for
    /// another schema, `state.model_mismatch` for another model, revision
    /// or dimension, and `limit.index_capacity` beyond 2^32 distinct
    /// embeddings in a field.
    pub fn extend(&mut self, other: Self) -> Result<()> {
        let offsets = self.offsets(&other)?;
        for ((field, more), offset) in self.fields.iter_mut().zip(other.fields).zip(offsets) {
            if let (Some(field), Some(more)) = (field, more) {
                field.data.extend_from_slice(&more.data);
                field.codes.extend_from_slice(&more.codes);
                field.bounds.extend_from_slice(&more.bounds);
                field
                    .rows
                    .extend(more.rows.iter().map(|row| row.map(|row| row + offset)));
            }
        }
        Ok(())
    }

    /// The row offset of `other`'s embeddings in each field once appended.
    fn offsets(&self, other: &Self) -> Result<Vec<u32>> {
        if other.schema != self.schema || other.fields.len() != self.fields.len() {
            return Err(Error::schema_mismatch());
        }
        if other.model != self.model {
            return Err(Error::model_mismatch(
                &self.model.name(),
                &other.model.name(),
            ));
        }
        let mut offsets = Vec::with_capacity(self.fields.len());
        for (field, more) in self.fields.iter().zip(&other.fields) {
            let offset = match (field, more) {
                (Some(field), Some(more)) => {
                    let rows = field.data.len() / field.dimension.max(1);
                    let total = rows + more.data.len() / more.dimension.max(1);
                    if total > MAX_ROWS {
                        return Err(Error::index_capacity(total, MAX_ROWS));
                    }
                    u32::try_from(rows).map_err(|_| Error::index_capacity(total, MAX_ROWS))?
                }
                (None, None) => 0,
                _ => return Err(Error::schema_mismatch()),
            };
            offsets.push(offset);
        }
        Ok(offsets)
    }

    pub fn heap_bytes(&self) -> usize {
        self.fields
            .iter()
            .flatten()
            .map(FieldVectors::heap_bytes)
            .sum()
    }
}

/// Whether the schema compares field `index` with embeddings.
pub fn is_embedded(schema: &Schema, index: usize) -> bool {
    matches!(
        schema.fields()[index].kind,
        FieldKind::Text(text) if text.mode == TextMode::Semantic
    )
}

/// Embeds the semantic text fields of `records` with one batch per field.
/// Identical normalized texts are encoded once.
///
/// # Errors
///
/// Returns `state.schema_mismatch` when a record was prepared with another
/// schema, `model.corrupt` when the embedder returns another number of rows
/// or another dimension than it declares, and the embedder's own errors.
pub fn embed_records(
    schema: &Schema,
    records: &[PreparedRecord],
    embedder: &dyn Embedder,
) -> Result<RecordVectors> {
    embed_records_cached(schema, records, embedder, None)
}

/// As [`embed_records`], reusing normalized semantic text embeddings.
///
/// # Errors
/// Also refuses an incompatible cache model, even when every text is cached.
pub fn embed_records_cached(
    schema: &Schema,
    records: &[PreparedRecord],
    embedder: &dyn Embedder,
    cache: Option<&EmbeddingCache>,
) -> Result<RecordVectors> {
    embed_records_selected(schema, records, embedder, cache, |_, _| true)
}

pub(crate) fn embed_records_selected(
    schema: &Schema,
    records: &[PreparedRecord],
    embedder: &dyn Embedder,
    cache: Option<&EmbeddingCache>,
    selected: impl Fn(usize, usize) -> bool,
) -> Result<RecordVectors> {
    if !records.iter().all(|record| record.is_prepared_with(schema)) {
        return Err(Error::schema_mismatch());
    }
    let model = embedder.descriptor().clone();
    if let Some(cache) = cache {
        cache.check_model(&model)?;
    }
    let dimension = model.dimension;
    if dimension == 0 {
        return Err(Error::corrupt_model(
            &model.name(),
            "zero embedding dimension",
        ));
    }
    let batch_rows = (MAX_BATCH_VALUES / dimension).clamp(1, 1024);
    let mut fields = Vec::with_capacity(schema.fields().len());
    for index in 0..schema.fields().len() {
        if !is_embedded(schema, index) {
            fields.push(None);
            continue;
        }
        // The distinct texts of the batch, and which one each record has.
        let mut distinct: Vec<&str> = Vec::new();
        let mut positions: HashMap<&str, usize> = HashMap::new();
        let texts: Vec<Option<usize>> = records
            .iter()
            .enumerate()
            .map(|(position, record)| match &record.fields()[index].state {
                FieldState::Text(text) if selected(position, index) && !text.trim().is_empty() => {
                    Some(*positions.entry(text).or_insert_with(|| {
                        distinct.push(text);
                        distinct.len() - 1
                    }))
                }
                _ => None,
            })
            .collect();
        let mut vectors = FieldVectors::new(dimension);
        let mut rows = Vec::with_capacity(distinct.len());
        // Bound temporary encoder matrices and Arc rows, including cold cache
        // misses. Only the final contiguous field buffer grows with the corpus.
        for chunk in distinct.chunks(batch_rows) {
            if embedder.descriptor() != &model {
                return Err(Error::model_mismatch(
                    &model.name(),
                    &embedder.descriptor().name(),
                ));
            }
            if let Some(cache) = cache {
                for row in cache.encode(chunk, embedder)? {
                    rows.push(row.map(|row| vectors.add_row(&row)).transpose()?);
                }
            } else {
                let embeddings = embedder.encode_batch(chunk)?;
                if embeddings.len() != chunk.len() || embeddings.dimension() != dimension {
                    return Err(Error::corrupt_model(
                        &model.name(),
                        &format!(
                            "the model returned {} vectors of {} dimensions for {} texts, expected {dimension} dimensions",
                            embeddings.len(),
                            embeddings.dimension(),
                            chunk.len()
                        ),
                    ));
                }
                let mut absent = embeddings.absent().iter().peekable();
                for position in 0..chunk.len() {
                    rows.push(if absent.next_if_eq(&&position).is_some() {
                        None
                    } else {
                        Some(vectors.add_row(embeddings.row(position))?)
                    });
                }
            }
        }
        vectors.rows = texts
            .into_iter()
            .map(|text| text.and_then(|position| rows[position]))
            .collect();
        fields.push(Some(vectors));
    }
    Ok(RecordVectors {
        schema: schema.preparation_fingerprint(),
        model,
        fields,
    })
}
