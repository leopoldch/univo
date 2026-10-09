//! Embeddings of the semantic text fields of prepared records, one
//! contiguous buffer per field. Hybrid fields are scored lexically and not
//! embedded (ADR 0009).

use std::collections::HashMap;

use univo_core::{Error, FieldKind, FieldState, PreparedRecord, Result, Schema, TextMode};
use univo_embed::{Embedder, ModelDescriptor};

/// Unit vectors of one field, by record in insertion order. Records with the
/// same text share one row; a record without an embedding, because its text
/// is missing or has no token the model knows, has none.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldVectors {
    dimension: usize,
    data: Vec<f32>,
    /// Row of each record in `data`, or `None`.
    rows: Vec<Option<u32>>,
}

impl FieldVectors {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension,
            data: Vec::new(),
            rows: Vec::new(),
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

    /// Bytes held by the embeddings and their row table.
    pub fn heap_bytes(&self) -> usize {
        self.data.capacity() * size_of::<f32>() + self.rows.capacity() * size_of::<Option<u32>>()
    }

    /// Stores a new row and returns its number.
    fn add_row(&mut self, vector: &[f32]) -> Result<u32> {
        debug_assert_eq!(vector.len(), self.dimension);
        let row = self.data.len() / self.dimension.max(1);
        let row = u32::try_from(row).map_err(|_| Error::index_capacity(row + 1, MAX_ROWS))?;
        self.data.extend_from_slice(vector);
        Ok(row)
    }
}

/// Distinct embeddings one field can hold.
const MAX_ROWS: usize = u32::MAX as usize;

/// Embeddings of every embedded field of a set of records, indexed by the
/// schema's field order, with the schema and the model that produced them.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordVectors {
    schema: u64,
    model: ModelDescriptor,
    fields: Vec<Option<FieldVectors>>,
}

impl RecordVectors {
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
        self.fields
            .iter()
            .map(|field| field.as_ref().and_then(|field| field.get(slot)))
            .collect()
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
    if !records.iter().all(|record| record.is_prepared_with(schema)) {
        return Err(Error::schema_mismatch());
    }
    let model = embedder.descriptor().clone();
    let dimension = model.dimension;
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
            .map(|record| match &record.fields()[index].state {
                FieldState::Text(text) if !text.trim().is_empty() => {
                    Some(*positions.entry(text).or_insert_with(|| {
                        distinct.push(text);
                        distinct.len() - 1
                    }))
                }
                _ => None,
            })
            .collect();
        let embeddings = embedder.encode_batch(&distinct)?;
        if embeddings.len() != distinct.len() || embeddings.dimension() != dimension {
            return Err(Error::corrupt_model(
                &model.name(),
                &format!(
                    "the model returned {} vectors of {} dimensions for {} texts, expected {dimension} dimensions",
                    embeddings.len(),
                    embeddings.dimension(),
                    distinct.len()
                ),
            ));
        }
        let mut vectors = FieldVectors::new(dimension);
        let mut absent = embeddings.absent().iter().peekable();
        let mut rows = Vec::with_capacity(distinct.len());
        for position in 0..distinct.len() {
            if absent.next_if_eq(&&position).is_some() {
                rows.push(None);
            } else {
                rows.push(Some(vectors.add_row(embeddings.row(position))?));
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
