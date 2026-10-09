//! Exact matches: equality of every selected field after normalization.

use std::hash::Hasher;

use rustc_hash::FxHasher;

use crate::error::{Error, Result};
use crate::fingerprint::{self, Trace};
use crate::normalize::{NormalizedValue, Transforms};
use crate::prepare::{FieldState, PreparedField, PreparedRecord};
use crate::schema::{ExactNormalizer, FieldKind, Schema};

/// Evidence that two records are an exact match.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExactMatch {
    /// Fields whose original values differ, with the normalization steps
    /// that were needed to make them equal. Empty when the selected
    /// originals are identical.
    pub normalized_fields: Vec<(usize, Transforms)>,
}

/// In-memory key of a normalized value: equal values have equal keys, and a
/// collision is never evidence. Not a persistent format.
pub fn value_fingerprint(value: &NormalizedValue) -> u64 {
    fingerprint::normalized(value)
}

/// In-memory key of a record's partition values: records of one partition
/// have equal keys, but a collision is never evidence, so candidates must
/// still be checked with [`PreparedRecord::partition`] equality. Not a
/// persistent format.
pub fn partition_fingerprint(record: &PreparedRecord) -> u64 {
    let mut hasher = FxHasher::default();
    hasher.write_usize(record.partition.len());
    for value in &record.partition {
        fingerprint::write_normalized(&mut hasher, value);
    }
    hasher.finish()
}

/// In-memory candidate key for a record's partition and selected fields.
///
/// IDs and normalization explanations do not participate. Types, missing
/// states and lengths do. Equal records have equal keys, but a key collision
/// is never evidence: every candidate must be checked with [`exact_match`].
/// This key is not a persistent format or a cryptographic digest.
///
/// # Errors
///
/// Returns `state.schema_mismatch` for a record prepared with another schema.
pub fn exact_fingerprint(schema: &Schema, record: &PreparedRecord) -> Result<u64> {
    if record.schema != schema.preparation_fingerprint() {
        return Err(Error::schema_mismatch());
    }
    let mut hasher = FxHasher::default();
    hasher.write_u64(record.schema);
    hasher.write_usize(record.partition.len());
    for value in &record.partition {
        fingerprint::write_normalized(&mut hasher, value);
    }
    hasher.write_usize(record.fields.len());
    for field in &record.fields {
        match &field.state {
            FieldState::Absent => hasher.write_u8(0),
            FieldState::Null => hasher.write_u8(1),
            FieldState::Text(text) => {
                hasher.write_u8(2);
                fingerprint::write_str(&mut hasher, text);
            }
            FieldState::Exact(value) => {
                hasher.write_u8(3);
                fingerprint::write_normalized(&mut hasher, value);
            }
        }
    }
    Ok(hasher.finish())
}

/// Whether two records are an exact match, as [`exact_match`] decides,
/// without building the explanation: no allocation, for candidate lanes
/// that only need the answer.
///
/// # Errors
///
/// As [`exact_match`].
pub fn is_exact_match(
    schema: &Schema,
    left: &PreparedRecord,
    right: &PreparedRecord,
) -> Result<bool> {
    let expected = schema.preparation_fingerprint();
    if left.schema != expected || right.schema != expected {
        return Err(Error::schema_mismatch());
    }
    if left.partition != right.partition {
        return Ok(false);
    }
    let mut informative = false;
    let pairs = left.fields.iter().zip(right.fields.iter());
    for (field, (a, b)) in schema.fields().iter().zip(pairs) {
        if a.state != b.state {
            return Ok(false);
        }
        informative |= is_informative(&field.kind, &a.state);
    }
    Ok(informative)
}

/// Returns the exact match between two records prepared with `schema`, if
/// any.
///
/// # Errors
///
/// Returns a `state.schema_mismatch` error if either record was prepared with a
/// schema whose [`Schema::preparation_fingerprint`] differs from `schema`'s.
///
/// Both records must share their partition, every field must be in the same
/// missing state with equal normalized values, and at least one informative
/// field must be present: identifiers and partition keys are not evidence.
pub fn exact_match(
    schema: &Schema,
    left: &PreparedRecord,
    right: &PreparedRecord,
) -> Result<Option<ExactMatch>> {
    let expected = schema.preparation_fingerprint();
    if left.schema != expected || right.schema != expected {
        return Err(Error::schema_mismatch());
    }
    if left.partition != right.partition {
        return Ok(None);
    }
    let mut informative = false;
    let mut normalized_fields = Vec::new();
    let pairs = left.fields.iter().zip(right.fields.iter());
    for (index, (field, (a, b))) in schema.fields().iter().zip(pairs).enumerate() {
        if a.state != b.state {
            return Ok(None);
        }
        informative |= is_informative(&field.kind, &a.state);
        if a.transforms.is_empty() && b.transforms.is_empty() {
            continue;
        }
        let steps = fingerprint::steps(&field.kind);
        let needed = fingerprint::needed_steps(steps, trace(&field.kind, a), trace(&field.kind, b));
        if !needed.is_empty() {
            normalized_fields.push((index, needed));
        }
    }
    Ok(informative.then_some(ExactMatch { normalized_fields }))
}

/// Fingerprints of every normalization stage of a field.
fn trace(kind: &FieldKind, field: &PreparedField) -> Trace {
    let normalized = match &field.state {
        FieldState::Text(text) => fingerprint::text(text),
        FieldState::Exact(NormalizedValue::Str(text)) if is_hostname(kind) => {
            fingerprint::text(text)
        }
        FieldState::Exact(value) => fingerprint::normalized(value),
        FieldState::Absent | FieldState::Null => 0,
    };
    Trace::complete(
        fingerprint::steps(kind),
        field.transforms,
        field.stages,
        normalized,
    )
}

fn is_hostname(kind: &FieldKind) -> bool {
    matches!(kind, FieldKind::Exact(exact) if exact.normalizer == ExactNormalizer::Hostname)
}

fn is_informative(kind: &FieldKind, state: &FieldState) -> bool {
    match (kind, state) {
        (_, FieldState::Absent | FieldState::Null) => false,
        (FieldKind::Text(_), FieldState::Text(text)) => !text.trim().is_empty(),
        (FieldKind::Exact(exact), FieldState::Exact(NormalizedValue::Str(text))) => {
            exact.normalizer != ExactNormalizer::Hostname || !text.trim().is_empty()
        }
        _ => true,
    }
}
