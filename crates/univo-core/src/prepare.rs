//! Validation and normalization of one record against a schema: steps 1 to 3
//! of the comparison pipeline in `specs/03-matching.md`.

use crate::acronym::AcronymKeys;
use crate::budget::Budget;
use crate::distinct::word_bits;
use crate::error::{Error, Result};
use crate::fingerprint::{self, Stages};
use crate::lexical;
use crate::normalize::{
    NormalizedValue, Transforms, normalize_hostname, normalize_text_observed, normalize_value,
    push_nfc,
};
use crate::path::FieldPath;
use crate::schema::{ExactNormalizer, FieldKind, Schema, Text, TextMode};
use crate::value::{Lookup, Value};

/// External identifier of a record; never normalized, typed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecordId {
    Str(Box<str>),
    Int(i64),
}

/// A record reduced to its selected, normalized fields.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRecord {
    /// [`Schema::preparation_fingerprint`] of the schema used.
    pub(crate) schema: u64,
    pub(crate) id: Option<RecordId>,
    pub(crate) partition: Box<[NormalizedValue]>,
    pub(crate) fields: Box<[PreparedField]>,
}

/// One compared field, in schema order.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedField {
    pub state: FieldState,
    /// Normalization steps that changed the original value.
    pub transforms: Transforms,
    /// Fingerprints before the steps in `transforms`, to explain exact
    /// matches.
    pub(crate) stages: Stages,
    /// Sorted character trigrams of normalized text compared lexically.
    pub(crate) trigrams: Option<Box<[u64]>>,
    /// Acronym and initials of normalized text, zero for other values.
    pub(crate) acronyms: AcronymKeys,
    /// [`word_bits`] of normalized text, zero for other values.
    pub(crate) word_bits: u64,
}

impl PreparedField {
    /// Sorted, deduplicated character trigrams of the normalized text, kept
    /// for text fields compared lexically.
    pub fn trigrams(&self) -> Option<&[u64]> {
        self.trigrams.as_deref()
    }

    /// Hashes of the acronym and initials of the normalized text.
    pub fn acronym_keys(&self) -> AcronymKeys {
        self.acronyms
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldState {
    Absent,
    Null,
    Text(Box<str>),
    Exact(NormalizedValue),
}

impl PreparedRecord {
    pub fn id(&self) -> Option<&RecordId> {
        self.id.as_ref()
    }

    pub fn partition(&self) -> &[NormalizedValue] {
        &self.partition
    }

    pub fn fields(&self) -> &[PreparedField] {
        &self.fields
    }

    /// Whether the record was prepared with a schema equivalent to `schema`.
    pub fn is_prepared_with(&self, schema: &Schema) -> bool {
        self.schema == schema.preparation_fingerprint()
    }
}

/// Validates `record` against `schema` and normalizes its selected fields.
///
/// Inputs are never coerced: a value of the wrong type is an error naming
/// the field path, not a string conversion. Every value is charged against
/// the resource limits before it is copied or normalized, so an oversized or
/// deeply nested input is rejected without being traversed beyond the limit.
///
/// # Errors
///
/// Returns an error of kind [`crate::ErrorKind::InvalidRecord`] when the
/// record has the wrong shape, a field has the wrong type, a hostname is
/// invalid, object keys collide, or a required field, the identifier or a
/// partition key is missing; of kind [`crate::ErrorKind::ResourceLimit`]
/// when a selected value is nested too deeply or the selected values,
/// identifier and partition keys included, are too large. The error names
/// the field path and carries a hint; see `docs/reference/errors.md`.
pub fn prepare(schema: &Schema, record: &Value) -> Result<PreparedRecord> {
    let root = FieldPath::root();
    let mut budget = Budget::new(schema.limits());
    if schema.compares_whole_record() {
        let Value::Str(text) = record else {
            return Err(Error::wrong_shape(&root, "a string", record.type_name()));
        };
        budget.charge_value(&root, record)?;
        let field = match &schema.fields()[0].kind {
            FieldKind::Text(options) => prepare_text(text, *options),
            FieldKind::Exact(_) => prepare_text(text, Text::default()),
        };
        return Ok(PreparedRecord {
            schema: schema.preparation_fingerprint(),
            id: None,
            partition: Box::new([]),
            fields: Box::new([field]),
        });
    }
    if !matches!(record, Value::Map(_)) {
        return Err(Error::wrong_shape(&root, "an object", record.type_name()));
    }

    let id = match schema.id_field() {
        Some(path) => Some(prepare_id(path, record.lookup(path)?, &mut budget)?),
        None => None,
    };
    let partition = schema
        .partition_by()
        .iter()
        .map(|path| prepare_partition(path, record.lookup(path)?, &mut budget))
        .collect::<Result<_>>()?;

    let mut fields = Vec::with_capacity(schema.fields().len());
    for field in schema.fields() {
        let value = match record.lookup(&field.path)? {
            Lookup::Present(value) if !matches!(value, Value::Null) => value,
            missing => {
                if field.kind.is_required() {
                    let null = matches!(missing, Lookup::Present(_));
                    return Err(Error::missing_required_field(&field.path, null));
                }
                let state = match missing {
                    Lookup::Absent => FieldState::Absent,
                    Lookup::Present(_) => FieldState::Null,
                };
                fields.push(PreparedField {
                    state,
                    transforms: Transforms::NONE,
                    stages: Stages::default(),
                    trigrams: None,
                    acronyms: AcronymKeys::default(),
                    word_bits: 0,
                });
                continue;
            }
        };
        if matches!(field.kind, FieldKind::Text(_)) && !matches!(value, Value::Str(_)) {
            return Err(Error::wrong_type(
                &field.path,
                "a string",
                value.type_name(),
            ));
        }
        budget.charge_value(&field.path, value)?;
        fields.push(match (&field.kind, value) {
            (FieldKind::Text(text), Value::Str(input)) => prepare_text(input, *text),
            (FieldKind::Exact(exact), _) => prepare_exact(&field.path, value, exact.normalizer)?,
            (FieldKind::Text(_), other) => {
                return Err(Error::wrong_type(
                    &field.path,
                    "a string",
                    other.type_name(),
                ));
            }
        });
    }

    Ok(PreparedRecord {
        schema: schema.preparation_fingerprint(),
        id,
        partition,
        fields: fields.into_boxed_slice(),
    })
}

fn prepare_id(path: &FieldPath, lookup: Lookup<'_>, budget: &mut Budget) -> Result<RecordId> {
    let value = match lookup {
        Lookup::Absent => return Err(Error::missing_identifier(path, false)),
        Lookup::Present(Value::Null) => return Err(Error::missing_identifier(path, true)),
        Lookup::Present(value) => value,
    };
    budget.charge_value(path, value)?;
    match value {
        Value::Str(id) => Ok(RecordId::Str(id.as_str().into())),
        Value::Int(id) => Ok(RecordId::Int(*id)),
        other => Err(Error::wrong_type(
            path,
            "a string or an integer",
            other.type_name(),
        )),
    }
}

fn prepare_partition(
    path: &FieldPath,
    lookup: Lookup<'_>,
    budget: &mut Budget,
) -> Result<NormalizedValue> {
    let value = match lookup {
        Lookup::Absent => return Err(Error::missing_partition_key(path, false)),
        Lookup::Present(Value::Null) => return Err(Error::missing_partition_key(path, true)),
        Lookup::Present(value @ (Value::List(_) | Value::Map(_))) => {
            return Err(Error::wrong_type(
                path,
                "a string, number or boolean",
                value.type_name(),
            ));
        }
        Lookup::Present(value) => value,
    };
    budget.charge_value(path, value)?;
    Ok(normalize_value(value)
        .map_err(|collision| Error::key_collision(path, &collision))?
        .0)
}

fn prepare_text(input: &str, options: Text) -> PreparedField {
    let mut out = String::with_capacity(input.len());
    let mut composed = None;
    let transforms = normalize_text_observed(input, options.case_fold, &mut out, |form| {
        composed = Some(fingerprint::text(form));
    });
    let mut stages = Stages::default();
    if transforms.contains(Transforms::UNICODE_FORM) {
        stages.0[0] = fingerprint::text(input);
    }
    if transforms.contains(Transforms::CASE) {
        stages.0[1] = composed.unwrap_or_else(|| fingerprint::text(input));
    }
    let trigrams = (options.mode != TextMode::Semantic).then(|| lexical::trigrams(&out));
    PreparedField {
        acronyms: AcronymKeys::of(&out),
        word_bits: word_bits(&out),
        state: FieldState::Text(out.into_boxed_str()),
        transforms,
        stages,
        trigrams,
    }
}

fn prepare_exact(
    path: &FieldPath,
    value: &Value,
    normalizer: ExactNormalizer,
) -> Result<PreparedField> {
    if normalizer == ExactNormalizer::Hostname {
        let Value::Str(input) = value else {
            return Err(Error::wrong_type(
                path,
                "a hostname string",
                value.type_name(),
            ));
        };
        let mut out = String::with_capacity(input.len());
        let transforms =
            normalize_hostname(input, &mut out).map_err(|_| Error::invalid_hostname(path))?;
        let stages = hostname_stages(input, &out, transforms);
        return Ok(PreparedField {
            state: FieldState::Exact(NormalizedValue::Str(out.into_boxed_str())),
            transforms,
            stages,
            trigrams: None,
            acronyms: AcronymKeys::default(),
            word_bits: 0,
        });
    }

    let (canonical, transforms) =
        normalize_value(value).map_err(|collision| Error::key_collision(path, &collision))?;
    let mut stages = Stages::default();
    if transforms.contains(Transforms::UNICODE_FORM) {
        stages.0[0] = fingerprint::value(value, false);
    }
    if transforms.contains(Transforms::KEY_ORDER) {
        stages.0[1] = fingerprint::value(value, true);
    }
    Ok(PreparedField {
        state: FieldState::Exact(canonical),
        transforms,
        stages,
        trigrams: None,
        acronyms: AcronymKeys::default(),
        word_bits: 0,
    })
}

fn hostname_stages(input: &str, out: &str, transforms: Transforms) -> Stages {
    let mut stages = Stages::default();
    if transforms.contains(Transforms::UNICODE_FORM) {
        stages.0[0] = fingerprint::text(input);
    }
    if transforms.contains(Transforms::CASE) {
        stages.0[1] = if transforms.contains(Transforms::UNICODE_FORM) {
            let mut composed = String::with_capacity(input.len());
            push_nfc(input, &mut composed);
            fingerprint::text(&composed)
        } else {
            fingerprint::text(input)
        };
    }
    if transforms.contains(Transforms::TRAILING_DOT) {
        stages.0[2] = fingerprint::text(&format!("{out}."));
    }
    stages
}
