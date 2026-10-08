//! Pairwise comparison of prepared records: a similarity per field, a
//! weighted score, its coverage, and the contradictions found. The rules are
//! in `docs/scoring.md`; scores rank pairs and are not probabilities.

use std::cmp::Ordering;

use crate::error::{Error, Result};
use crate::lexical;
use crate::normalize::NormalizedValue;
use crate::prepare::{FieldState, PreparedField, PreparedRecord};
use crate::schema::{Conflict, ExactNormalizer, Field, FieldKind, Schema, TextMode};

/// Version of the comparison rules. Any change in a similarity, a signal or
/// the score for some pair must increment it.
pub const SCORING_VERSION: u32 = 1;

/// What produced a field's similarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Signal {
    /// At least one side is absent, null or blank.
    Missing,
    /// Text compared by normalized edit distance, the stronger lexical signal.
    EditDistance,
    /// Text compared by character trigram overlap, the stronger lexical signal.
    Trigrams,
    /// Values compared by typed equality after normalization.
    Exact,
    /// Objects compared key by key, on the keys present on both sides.
    Keys,
    /// Text compared semantically only; no similarity until embeddings.
    Semantic,
}

/// Comparison of one field, in schema order.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldComparison {
    /// Similarity in `[0, 1]`, or `None` when the field gives no evidence.
    pub similarity: Option<f32>,
    pub signal: Signal,
    /// Present values that differ, on an exact field or on shared object keys.
    pub contradiction: bool,
    /// The contradiction is on a discriminant field and forbids a match.
    pub veto: bool,
    /// Evidence present on one side only: a missing value, or object keys on
    /// one side.
    pub incomplete: bool,
}

/// Result of comparing two records of the same partition.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// Weighted mean of the field similarities, over the fields that give
    /// evidence; `None` when no field does, never 1 by default.
    pub score: Option<f32>,
    /// Share of the total weight carried by the fields that give evidence.
    pub coverage: f32,
    /// A discriminant field contradicts: the pair must not match, whatever
    /// its score.
    pub vetoed: bool,
    pub fields: Box<[FieldComparison]>,
}

/// Compares two records prepared with `schema`.
///
/// Returns `Ok(None)` for records of different partitions, which are never
/// compared.
///
/// # Errors
///
/// Returns a `state.schema_mismatch` error if either record was prepared
/// with a schema whose preparation fingerprint differs from `schema`'s.
pub fn compare(
    schema: &Schema,
    left: &PreparedRecord,
    right: &PreparedRecord,
) -> Result<Option<Comparison>> {
    let expected = schema.preparation_fingerprint();
    if left.schema != expected || right.schema != expected {
        return Err(Error::schema_mismatch());
    }
    if left.partition != right.partition {
        return Ok(None);
    }

    let fields: Box<[FieldComparison]> = schema
        .fields()
        .iter()
        .zip(left.fields.iter().zip(right.fields.iter()))
        .map(|(field, (a, b))| compare_field(field, a, b))
        .collect();

    let mut total = 0.0f64;
    let mut evidence = 0.0f64;
    let mut weighted = 0.0f64;
    for (field, comparison) in schema.fields().iter().zip(fields.iter()) {
        let weight = f64::from(field.kind.weight());
        total += weight;
        if let Some(similarity) = comparison.similarity {
            evidence += weight;
            weighted += weight * f64::from(similarity);
        }
    }
    #[allow(clippy::cast_possible_truncation)]
    let score = (evidence > 0.0).then(|| (weighted / evidence) as f32);
    #[allow(clippy::cast_possible_truncation)]
    let coverage = if total > 0.0 {
        (evidence / total) as f32
    } else {
        0.0
    };
    Ok(Some(Comparison {
        score,
        coverage,
        vetoed: fields.iter().any(|field| field.veto),
        fields,
    }))
}

fn missing(incomplete: bool) -> FieldComparison {
    FieldComparison {
        similarity: None,
        signal: Signal::Missing,
        contradiction: false,
        veto: false,
        incomplete,
    }
}

fn compare_field(field: &Field, left: &PreparedField, right: &PreparedField) -> FieldComparison {
    let (left_present, right_present) = (is_present(field, left), is_present(field, right));
    if !(left_present && right_present) {
        return missing(left_present || right_present);
    }
    match (&field.kind, &left.state, &right.state) {
        (FieldKind::Text(text), FieldState::Text(a), FieldState::Text(b)) => {
            compare_text(text.mode, a, b, left, right)
        }
        (FieldKind::Exact(exact), FieldState::Exact(a), FieldState::Exact(b)) => {
            compare_exact(exact.conflict, a, b)
        }
        _ => missing(false),
    }
}

/// Absent, null and blank values give no evidence.
fn is_present(field: &Field, prepared: &PreparedField) -> bool {
    match (&field.kind, &prepared.state) {
        (_, FieldState::Absent | FieldState::Null) => false,
        (FieldKind::Text(_), FieldState::Text(text)) => !text.trim().is_empty(),
        (FieldKind::Exact(exact), FieldState::Exact(NormalizedValue::Str(text)))
            if exact.normalizer == ExactNormalizer::Hostname =>
        {
            !text.trim().is_empty()
        }
        _ => true,
    }
}

fn compare_text(
    mode: TextMode,
    left: &str,
    right: &str,
    left_field: &PreparedField,
    right_field: &PreparedField,
) -> FieldComparison {
    let lexical = match (&left_field.trigrams, &right_field.trigrams) {
        (Some(a), Some(b)) if mode != TextMode::Semantic => Some((a, b)),
        _ => None,
    };
    let Some((left_trigrams, right_trigrams)) = lexical else {
        return FieldComparison {
            similarity: None,
            signal: Signal::Semantic,
            contradiction: false,
            veto: false,
            incomplete: false,
        };
    };
    let trigrams = lexical::jaccard(left_trigrams, right_trigrams);
    let (similarity, signal) = match lexical::edit_similarity(left, right) {
        Some(edit) if edit >= trigrams => (edit, Signal::EditDistance),
        _ => (trigrams, Signal::Trigrams),
    };
    FieldComparison {
        similarity: Some(similarity),
        signal,
        contradiction: false,
        veto: false,
        incomplete: false,
    }
}

fn compare_exact(
    conflict: Conflict,
    left: &NormalizedValue,
    right: &NormalizedValue,
) -> FieldComparison {
    let (similarity, signal, contradiction, incomplete) = match (left, right) {
        (NormalizedValue::Map(a), NormalizedValue::Map(b)) => {
            let keys = compare_keys(a, b);
            let similarity = (keys.shared > 0).then(|| ratio(keys.equal, keys.shared));
            (
                similarity,
                Signal::Keys,
                keys.equal < keys.shared,
                keys.one_sided,
            )
        }
        _ if left == right => (Some(1.0), Signal::Exact, false, false),
        _ => (Some(0.0), Signal::Exact, true, false),
    };
    FieldComparison {
        similarity,
        signal,
        contradiction,
        veto: contradiction && conflict == Conflict::Reject,
        incomplete,
    }
}

struct KeyComparison {
    /// Keys whose values are present, not null, on both sides.
    shared: usize,
    /// Shared keys with equal values.
    equal: usize,
    /// Some key is present on one side only.
    one_sided: bool,
}

/// Merges two key-sorted objects. A null value is unknown, like an absent
/// key: it never contradicts, it only makes the comparison incomplete.
fn compare_keys(
    left: &[(Box<str>, NormalizedValue)],
    right: &[(Box<str>, NormalizedValue)],
) -> KeyComparison {
    let (left, right) = (present_entries(left), present_entries(right));
    let mut keys = KeyComparison {
        shared: 0,
        equal: 0,
        one_sided: false,
    };
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        match left[i].0.cmp(&right[j].0) {
            Ordering::Less => {
                keys.one_sided = true;
                i += 1;
            }
            Ordering::Greater => {
                keys.one_sided = true;
                j += 1;
            }
            Ordering::Equal => {
                keys.shared += 1;
                keys.equal += usize::from(left[i].1 == right[j].1);
                i += 1;
                j += 1;
            }
        }
    }
    keys.one_sided |= i < left.len() || j < right.len();
    keys
}

fn present_entries(entries: &[(Box<str>, NormalizedValue)]) -> Vec<&(Box<str>, NormalizedValue)> {
    entries
        .iter()
        .filter(|(_, value)| *value != NormalizedValue::Null)
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn ratio(numerator: usize, denominator: usize) -> f32 {
    numerator as f32 / denominator as f32
}
