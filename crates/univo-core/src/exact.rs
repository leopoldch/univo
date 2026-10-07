//! Exact matches: equality of every selected field after normalization.

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

/// Returns the exact match between two records prepared with `schema`, if
/// any.
///
/// # Errors
///
/// Returns [`Error::SchemaMismatch`] if either record was prepared with a
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
        return Err(Error::SchemaMismatch);
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
