//! Validation and normalization of one record against a schema: steps 1 to 3
//! of the comparison pipeline in `specs/03-matching.md`.

use crate::error::{Error, Resource, Result};
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
/// the field path, not a string conversion.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] when the record has the wrong shape, a
/// field has the wrong type, or a required field, the identifier or a
/// partition key is missing; [`Error::ResourceLimit`] when a selected value
/// is nested too deeply or the selected fields are too large.
pub fn prepare(schema: &Schema, record: &Value) -> Result<PreparedRecord> {
    let root = FieldPath::root();
    if schema.compares_whole_record() {
        let Value::Str(text) = record else {
            return Err(type_error(&root, "a string", record));
        };
        let case_fold = match &schema.fields()[0].kind {
            FieldKind::Text(text) => text.case_fold,
            FieldKind::Exact(_) => false,
        };
        let field = prepare_text(text, case_fold);
        check_size(&root, record.selected_bytes(), schema)?;
        return Ok(PreparedRecord {
            id: None,
            partition: Box::new([]),
            fields: Box::new([field]),
        });
    }
    if !matches!(record, Value::Map(_)) {
        return Err(type_error(&root, "an object", record));
    }

    let id = schema
        .id_field()
        .map(|path| prepare_id(path, record.lookup(path)?))
        .transpose()?;
    let partition = schema
        .partition_by()
        .iter()
        .map(|path| prepare_partition(path, record.lookup(path)?))
        .collect::<Result<_>>()?;

    let limits = schema.limits();
    let mut selected_bytes = 0;
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
        let depth = value.depth();
        if depth > limits.max_depth {
            return Err(Error::ResourceLimit {
                path: field.path.clone(),
                resource: Resource::NestingDepth,
                limit: limits.max_depth,
                actual: depth,
            });
        }
        selected_bytes += value.selected_bytes();
        fields.push(match &field.kind {
            FieldKind::Text(text) => match value {
                Value::Str(input) => prepare_text(input, text.case_fold),
                other => return Err(type_error(&field.path, "a string", other)),
            },
            FieldKind::Exact(exact) => prepare_exact(&field.path, value, exact.normalizer)?,
        });
    }
    check_size(&root, selected_bytes, schema)?;

    Ok(PreparedRecord {
        id,
        partition,
        fields: fields.into_boxed_slice(),
    })
}

fn prepare_id(path: &FieldPath, lookup: Lookup<'_>) -> Result<RecordId> {
    match lookup {
        Lookup::Present(Value::Str(id)) => Ok(RecordId::Str(id.as_str().into())),
        Lookup::Present(Value::Int(id)) => Ok(RecordId::Int(*id)),
        Lookup::Absent | Lookup::Present(Value::Null) => {
            Err(Error::record(path, "identifier is missing"))
        }
        Lookup::Present(other) => Err(type_error(path, "a string or an integer", other)),
    }
}

fn prepare_partition(path: &FieldPath, lookup: Lookup<'_>) -> Result<NormalizedValue> {
    match lookup {
        Lookup::Absent | Lookup::Present(Value::Null) => {
            Err(Error::record(path, "partition key is missing"))
        }
        Lookup::Present(value @ (Value::List(_) | Value::Map(_))) => {
            Err(type_error(path, "a scalar", value))
        }
        Lookup::Present(value) => Ok(normalize_value(value)
            .map_err(|_| Error::record(path, "invalid partition key"))?
            .0),
    }
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
        let transforms = normalize_hostname(input, &mut out);
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

fn check_size(path: &FieldPath, selected_bytes: usize, schema: &Schema) -> Result<()> {
    let limit = schema.limits().max_selected_bytes;
    if selected_bytes > limit {
        return Err(Error::ResourceLimit {
            path: path.clone(),
            resource: Resource::SelectedBytes,
            limit,
            actual: selected_bytes,
        });
    }
    Ok(())
}

fn type_error(path: &FieldPath, expected: &str, found: &Value) -> Error {
    Error::record(
        path,
        format!("expected {expected}, found {}", found.type_name()),
    )
}
