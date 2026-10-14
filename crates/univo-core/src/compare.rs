//! Pairwise comparison of prepared records: a similarity per field, a
//! weighted score, its coverage, and the contradictions found. The rules are
//! in `docs/scoring.md`; scores rank pairs and are not probabilities.

use std::cmp::Ordering;

use crate::acronym::{ACRONYM_SIMILARITY, acronym_similarity};
use crate::error::{Error, Result};
use crate::lexical;
use crate::normalize::NormalizedValue;
use crate::prepare::{FieldState, PreparedField, PreparedRecord};
use crate::schema::{Conflict, ExactNormalizer, Field, FieldKind, Schema, TextMode};
use crate::semantic::semantic_similarity;

/// Version of the comparison rules. Any change in a similarity, a signal or
/// the score for some pair must increment it.
///
/// 2: semantic text fields compared by the cosine of their embeddings.
/// 3: a text spelling the initials of the other's words scores at least
/// [`ACRONYM_SIMILARITY`], in every text mode (ADR 0022).
pub const SCORING_VERSION: u32 = 3;

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
    /// Text of a semantic field, compared by the cosine of its embeddings;
    /// without both embeddings, no similarity.
    Semantic,
    /// One text is the acronym of the other's words, the stronger signal.
    Acronym,
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
    /// Share of the field's information present on both sides: 1 for a
    /// compared scalar or text, shared keys over shared and one-sided keys
    /// for objects, 0 without evidence. It weights the field in the score and
    /// the coverage.
    pub evidence: f32,
}

/// Result of comparing two records of the same partition.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// Mean of the field similarities weighted by weight times evidence;
    /// `None` when no field gives evidence, never 1 by default.
    pub score: Option<f32>,
    /// Share of the total weight backed by evidence: `Σ w·e / Σ w`.
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
    compare_with_vectors(schema, left, &[], right, &[])
}

/// Compares two records prepared with `schema`, using the embeddings of
/// their semantic text fields: `vectors[i]` is the unit vector of field
/// `i`, or `None` when the field is not embedded or its text has no
/// embedding. A shorter slice leaves the remaining fields without
/// embeddings; vectors of other fields are ignored.
///
/// # Errors
///
/// As [`compare`].
pub fn compare_with_vectors(
    schema: &Schema,
    left: &PreparedRecord,
    left_vectors: &[Option<&[f32]>],
    right: &PreparedRecord,
    right_vectors: &[Option<&[f32]>],
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
        .enumerate()
        .map(|(index, (field, (a, b)))| {
            let vectors = field_vector(left_vectors, index).zip(field_vector(right_vectors, index));
            compare_field(field, a, b, vectors)
        })
        .collect();

    let mut total = 0.0f64;
    let mut evidence = 0.0f64;
    let mut weighted = 0.0f64;
    for (field, comparison) in schema.fields().iter().zip(fields.iter()) {
        let weight = f64::from(field.kind.weight());
        total += weight;
        if let Some(similarity) = comparison.similarity {
            let backed = weight * f64::from(comparison.evidence);
            evidence += backed;
            weighted += backed * f64::from(similarity);
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

fn field_vector<'a>(vectors: &[Option<&'a [f32]>], index: usize) -> Option<&'a [f32]> {
    vectors.get(index).copied().flatten()
}

fn missing(incomplete: bool) -> FieldComparison {
    FieldComparison {
        similarity: None,
        signal: Signal::Missing,
        contradiction: false,
        veto: false,
        incomplete,
        evidence: 0.0,
    }
}

fn compare_field(
    field: &Field,
    left: &PreparedField,
    right: &PreparedField,
    vectors: Option<(&[f32], &[f32])>,
) -> FieldComparison {
    let (left_present, right_present) = (has_value(field, left), has_value(field, right));
    if !(left_present && right_present) {
        return missing(left_present || right_present);
    }
    match (&field.kind, &left.state, &right.state) {
        (FieldKind::Text(text), FieldState::Text(a), FieldState::Text(b)) => {
            compare_text(text.mode, a, b, left, right, vectors)
        }
        (FieldKind::Exact(exact), FieldState::Exact(a), FieldState::Exact(b)) => {
            compare_exact(exact.conflict, a, b)
        }
        _ => missing(false),
    }
}

/// Whether a prepared field holds a value that can give evidence: absent,
/// null and blank values never do.
pub fn has_value(field: &Field, prepared: &PreparedField) -> bool {
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
    vectors: Option<(&[f32], &[f32])>,
) -> FieldComparison {
    let lexical = match (mode, &left_field.trigrams, &right_field.trigrams) {
        (TextMode::Lexical | TextMode::Hybrid, Some(a), Some(b)) => {
            Some(lexical_similarity(left, a, right, b))
        }
        _ => None,
    };
    let semantic = match (mode, vectors) {
        (TextMode::Semantic, Some((a, b))) if a.len() == b.len() => {
            Some((semantic_similarity(a, b), Signal::Semantic))
        }
        _ => None,
    };
    // Hybrid fields are scored lexically: on the critical corpus, adding
    // their cosine lowered the ranking quality (ADR 0009).
    let mut best = lexical.or(semantic);
    if best.is_none_or(|(similarity, _)| similarity < ACRONYM_SIMILARITY)
        && left_field.acronyms.may_match(&right_field.acronyms)
        && let Some(acronym) = acronym_similarity(left, right)
    {
        best = Some((acronym, Signal::Acronym));
    }
    let (similarity, signal, evidence) = match best {
        Some((similarity, signal)) => (Some(similarity), signal, 1.0),
        None => (None, Signal::Semantic, 0.0),
    };
    FieldComparison {
        similarity,
        signal,
        contradiction: false,
        veto: false,
        incomplete: false,
        evidence,
    }
}

/// Similarity of two normalized texts compared lexically, with their sorted
/// trigrams: the larger of the edit similarity, for strings of at most
/// [`lexical::MAX_EDIT_CHARS`] characters, and the trigram Jaccard index.
pub fn lexical_similarity(
    left: &str,
    left_trigrams: &[u64],
    right: &str,
    right_trigrams: &[u64],
) -> (f32, Signal) {
    let trigrams = lexical::jaccard(left_trigrams, right_trigrams);
    match lexical::edit_similarity(left, right) {
        Some(edit) if edit >= trigrams => (edit, Signal::EditDistance),
        _ => (trigrams, Signal::Trigrams),
    }
}

fn compare_exact(
    conflict: Conflict,
    left: &NormalizedValue,
    right: &NormalizedValue,
) -> FieldComparison {
    let (similarity, signal, contradiction, incomplete, evidence) = match (left, right) {
        (NormalizedValue::Map(a), NormalizedValue::Map(b)) => {
            let mut keys = KeyComparison::default();
            keys.merge(a, b);
            if keys.shared == 0 && keys.one_sided == 0 && keys.unknown == 0 {
                // Two empty objects: equal values.
                (Some(1.0), Signal::Keys, false, false, 1.0)
            } else {
                let similarity = (keys.shared > 0).then(|| ratio(keys.equal, keys.shared));
                let known = keys.shared + keys.one_sided;
                let evidence = if known == 0 {
                    0.0
                } else {
                    ratio(keys.shared, known)
                };
                let contradiction = keys.equal < keys.shared;
                (
                    similarity,
                    Signal::Keys,
                    contradiction,
                    keys.one_sided > 0,
                    evidence,
                )
            }
        }
        _ if left == right => (Some(1.0), Signal::Exact, false, false, 1.0),
        _ => (Some(0.0), Signal::Exact, true, false, 1.0),
    };
    FieldComparison {
        similarity,
        signal,
        contradiction,
        veto: contradiction && conflict == Conflict::Reject,
        incomplete,
        evidence,
    }
}

/// Leaf counts of a key-by-key comparison of two objects.
#[derive(Default)]
struct KeyComparison {
    /// Leaves present, not null, on both sides.
    shared: usize,
    /// Shared leaves with equal values.
    equal: usize,
    /// Keys present on one side only, each counted once whatever its value.
    one_sided: usize,
    /// Null values skipped, on either side: unknown, never evidence.
    unknown: usize,
}

impl KeyComparison {
    /// Merges two key-sorted objects. Null is unknown at every depth, like
    /// an absent key: it never contradicts, it only makes the comparison
    /// incomplete. Nested objects present on both sides are compared key by
    /// key; any other value is a leaf compared by typed equality. Recursion
    /// follows nesting, which preparation bounds by `max_depth`.
    fn merge(
        &mut self,
        left: &[(Box<str>, NormalizedValue)],
        right: &[(Box<str>, NormalizedValue)],
    ) {
        let is_null = |entry: &&(Box<str>, NormalizedValue)| entry.1 == NormalizedValue::Null;
        self.unknown += left.iter().filter(is_null).count() + right.iter().filter(is_null).count();
        let not_null = |entry: &&(Box<str>, NormalizedValue)| entry.1 != NormalizedValue::Null;
        let mut left = left.iter().filter(not_null).peekable();
        let mut right = right.iter().filter(not_null).peekable();
        loop {
            match (left.peek(), right.peek()) {
                (None, None) => return,
                (Some(_), None) => {
                    self.one_sided += left.count();
                    return;
                }
                (None, Some(_)) => {
                    self.one_sided += right.count();
                    return;
                }
                (Some((a_key, a)), Some((b_key, b))) => match a_key.cmp(b_key) {
                    Ordering::Less => {
                        self.one_sided += 1;
                        left.next();
                    }
                    Ordering::Greater => {
                        self.one_sided += 1;
                        right.next();
                    }
                    Ordering::Equal => {
                        match (a, b) {
                            // Two empty objects are an equal leaf, not an absence.
                            (NormalizedValue::Map(a), NormalizedValue::Map(b))
                                if !(a.is_empty() && b.is_empty()) =>
                            {
                                self.merge(a, b);
                            }
                            _ => {
                                self.shared += 1;
                                self.equal += usize::from(a == b);
                            }
                        }
                        left.next();
                        right.next();
                    }
                },
            }
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn ratio(numerator: usize, denominator: usize) -> f32 {
    numerator as f32 / denominator as f32
}
