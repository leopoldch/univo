//! Embeddings of the semantic text fields of prepared records, one
//! contiguous buffer per field. Hybrid fields are scored lexically and not
//! embedded (ADR 0009).

use std::collections::HashMap;

use univo_core::{Error, FieldKind, FieldState, PreparedRecord, Result, Schema, TextMode};
use univo_embed::Embedder;

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
/// schema's field order.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordVectors {
    fields: Vec<Option<FieldVectors>>,
}

impl RecordVectors {
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

    /// Appends the embeddings of later records.
    ///
    /// # Errors
    ///
    /// Returns `limit.index_capacity` beyond 2^32 distinct embeddings in a
    /// field, leaving `self` unchanged.
    pub fn extend(&mut self, other: Self) -> Result<()> {
        for (field, more) in self.fields.iter().zip(&other.fields) {
            if let (Some(field), Some(more)) = (field, more) {
                let rows = (field.data.len() + more.data.len()) / field.dimension.max(1);
                if rows > MAX_ROWS {
                    return Err(Error::index_capacity(rows, MAX_ROWS));
                }
            }
        }
        for (field, more) in self.fields.iter_mut().zip(other.fields) {
            if let (Some(field), Some(more)) = (field, more) {
                let offset = u32::try_from(field.data.len() / field.dimension.max(1))
                    .map_err(|_| Error::index_capacity(MAX_ROWS + 1, MAX_ROWS))?;
                field.data.extend_from_slice(&more.data);
                field
                    .rows
                    .extend(more.rows.iter().map(|row| row.map(|row| row + offset)));
            }
        }
        Ok(())
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
/// Returns the embedder's errors, such as `model.corrupt`.
pub fn embed_records(
    schema: &Schema,
    records: &[PreparedRecord],
    embedder: &dyn Embedder,
) -> Result<RecordVectors> {
    let dimension = embedder.descriptor().dimension;
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
    Ok(RecordVectors { fields })
}
