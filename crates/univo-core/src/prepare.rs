//! Validation and normalization of one record against a schema: steps 1 to 3
//! of the comparison pipeline in `specs/03-matching.md`.

use crate::budget::Budget;
use crate::error::{Error, Result};
use crate::fingerprint::{self, Stages};
use crate::normalize::{
    NormalizedValue, Transforms, normalize_hostname, normalize_text_observed, normalize_value,
    push_nfc,
};
use crate::path::FieldPath;
use crate::schema::{ExactNormalizer, FieldKind, Schema};
use crate::value::{Lookup, Value};

/// External identifier of a record; never normalized, typed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
/// Returns [`Error::InvalidRecord`] when the record has the wrong shape, a
/// field has the wrong type, a hostname is invalid, or a required field, the
/// identifier or a partition key is missing; [`Error::ResourceLimit`] when a
/// selected value is nested too deeply or the selected values, identifier
/// and partition keys included, are too large.
pub fn prepare(schema: &Schema, record: &Value) -> Result<PreparedRecord> {
    let root = FieldPath::root();
    let mut budget = Budget::new(schema.limits());
    if schema.compares_whole_record() {
        let Value::Str(text) = record else {
            return Err(type_error(&root, "a string", record));
        };
        budget.charge_value(&root, record)?;
        let case_fold = match &schema.fields()[0].kind {
            FieldKind::Text(text) => text.case_fold,
            FieldKind::Exact(_) => false,
        };
        return Ok(PreparedRecord {
            schema: schema.preparation_fingerprint(),
            id: None,
            partition: Box::new([]),
            fields: Box::new([prepare_text(text, case_fold)]),
        });
    }
    if !matches!(record, Value::Map(_)) {
        return Err(type_error(&root, "an object", record));
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
                    return Err(Error::record(&field.path, "required field is missing"));
                }
                let state = match missing {
                    Lookup::Absent => FieldState::Absent,
                    Lookup::Present(_) => FieldState::Null,
                };
                fields.push(PreparedField {
                    state,
                    transforms: Transforms::NONE,
                    stages: Stages::default(),
                });
                continue;
            }
        };
        if matches!(field.kind, FieldKind::Text(_)) && !matches!(value, Value::Str(_)) {
            return Err(type_error(&field.path, "a string", value));
        }
        budget.charge_value(&field.path, value)?;
        fields.push(match (&field.kind, value) {
            (FieldKind::Text(text), Value::Str(input)) => prepare_text(input, text.case_fold),
            (FieldKind::Exact(exact), _) => prepare_exact(&field.path, value, exact.normalizer)?,
            (FieldKind::Text(_), other) => return Err(type_error(&field.path, "a string", other)),
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
        Lookup::Absent | Lookup::Present(Value::Null) => {
            return Err(Error::record(path, "identifier is missing"));
        }
        Lookup::Present(value) => value,
    };
    budget.charge_value(path, value)?;
    match value {
        Value::Str(id) => Ok(RecordId::Str(id.as_str().into())),
        Value::Int(id) => Ok(RecordId::Int(*id)),
        other => Err(type_error(path, "a string or an integer", other)),
    }
}

fn prepare_partition(
    path: &FieldPath,
    lookup: Lookup<'_>,
    budget: &mut Budget,
) -> Result<NormalizedValue> {
    let value = match lookup {
        Lookup::Absent | Lookup::Present(Value::Null) => {
            return Err(Error::record(path, "partition key is missing"));
        }
        Lookup::Present(value @ (Value::List(_) | Value::Map(_))) => {
            return Err(type_error(path, "a scalar", value));
        }
        Lookup::Present(value) => value,
    };
    budget.charge_value(path, value)?;
    Ok(normalize_value(value)
        .map_err(|_| Error::record(path, "invalid partition key"))?
        .0)
}

fn prepare_text(input: &str, case_fold: bool) -> PreparedField {
    let mut out = String::with_capacity(input.len());
    let mut composed = None;
    let transforms = normalize_text_observed(input, case_fold, &mut out, |form| {
        composed = Some(fingerprint::text(form));
    });
    let mut stages = Stages::default();
    if transforms.contains(Transforms::UNICODE_FORM) {
        stages.0[0] = fingerprint::text(input);
    }
    if transforms.contains(Transforms::CASE) {
        stages.0[1] = composed.unwrap_or_else(|| fingerprint::text(input));
    }
    PreparedField {
        state: FieldState::Text(out.into_boxed_str()),
        transforms,
        stages,
    }
}

fn prepare_exact(
    path: &FieldPath,
    value: &Value,
    normalizer: ExactNormalizer,
) -> Result<PreparedField> {
    if normalizer == ExactNormalizer::Hostname {
        let Value::Str(input) = value else {
            return Err(type_error(path, "a hostname string", value));
        };
        let mut out = String::with_capacity(input.len());
        let transforms = normalize_hostname(input, &mut out)
            .map_err(|_| Error::record(path, "hostname ends with more than one dot"))?;
        let stages = hostname_stages(input, &out, transforms);
        return Ok(PreparedField {
            state: FieldState::Exact(NormalizedValue::Str(out.into_boxed_str())),
            transforms,
            stages,
        });
    }

    let (canonical, transforms) = normalize_value(value).map_err(|collision| {
        Error::record(
            path,
            format!(
                "object keys collide after Unicode normalization: {}",
                collision.0
            ),
        )
    })?;
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

fn type_error(path: &FieldPath, expected: &str, found: &Value) -> Error {
    Error::record(
        path,
        format!("expected {expected}, found {}", found.type_name()),
    )
}
