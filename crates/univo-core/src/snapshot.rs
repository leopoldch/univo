//! Explicit version-1 prepared-record wire format. No indexes or original
//! unselected data are serialized; trigram postings are rebuilt on load.
use std::io::{Read, Write};

use crate::fingerprint::Stages;
use crate::normalize::Transforms;
use crate::{
    Error, FieldKind, FieldState, Map, NormalizedValue, PreparedField, PreparedRecord, RecordId,
    Result, Schema, TextMode, Value, normalize_hostname, normalize_text, normalize_value,
};

fn invalid() -> Error {
    Error::snapshot_invalid("invalid prepared record")
}
fn write(out: &mut impl Write, bytes: &[u8]) -> Result<()> {
    out.write_all(bytes).map_err(|e| Error::snapshot_io(&e))
}
fn length(out: &mut impl Write, n: usize) -> Result<()> {
    let n = u64::try_from(n).map_err(|_| invalid())?;
    write(out, &n.to_le_bytes())
}
fn string(out: &mut impl Write, s: &str) -> Result<()> {
    length(out, s.len())?;
    write(out, s.as_bytes())
}
fn value(out: &mut impl Write, v: &NormalizedValue) -> Result<()> {
    match v {
        NormalizedValue::Null => write(out, &[0]),
        NormalizedValue::Bool(v) => write(out, &[1, u8::from(*v)]),
        NormalizedValue::Int(v) => {
            write(out, &[2])?;
            write(out, &v.to_le_bytes())
        }
        NormalizedValue::Float(v) => {
            write(out, &[3])?;
            write(out, &v.get().to_le_bytes())
        }
        NormalizedValue::Str(v) => {
            write(out, &[4])?;
            string(out, v)
        }
        NormalizedValue::List(v) => {
            write(out, &[5])?;
            length(out, v.len())?;
            for child in v {
                value(out, child)?;
            }
            Ok(())
        }
        NormalizedValue::Map(v) => {
            write(out, &[6])?;
            length(out, v.len())?;
            for (key, child) in v {
                string(out, key)?;
                value(out, child)?;
            }
            Ok(())
        }
    }
}

/// Writes one prepared record in the version-1 little-endian format.
/// # Errors
/// Returns snapshot I/O errors without changing the record.
pub fn write_record(out: &mut impl Write, record: &PreparedRecord) -> Result<()> {
    match record.id() {
        None => write(out, &[0])?,
        Some(RecordId::Int(id)) => {
            write(out, &[1])?;
            write(out, &id.to_le_bytes())?;
        }
        Some(RecordId::Str(id)) => {
            write(out, &[2])?;
            string(out, id)?;
        }
    }
    for partition in record.partition() {
        value(out, partition)?;
    }
    for field in record.fields() {
        match &field.state {
            FieldState::Absent => write(out, &[0])?,
            FieldState::Null => write(out, &[1])?,
            FieldState::Text(text) => {
                write(out, &[2])?;
                string(out, text)?;
            }
            FieldState::Exact(v) => {
                write(out, &[3])?;
                value(out, v)?;
            }
        }
        let flags = [
            Transforms::UNICODE_FORM,
            Transforms::CASE,
            Transforms::TRAILING_DOT,
            Transforms::KEY_ORDER,
        ]
        .iter()
        .enumerate()
        .fold(0u8, |bits, (i, flag)| {
            bits | (u8::from(field.transforms.contains(*flag)) << i)
        });
        write(out, &[flags])?;
        for stage in field.stages.0 {
            write(out, &stage.to_le_bytes())?;
        }
    }
    Ok(())
}

struct Reader<'a, R> {
    input: &'a mut R,
    left: usize,
    depth: usize,
    values_left: usize,
}
impl<R: Read> Reader<'_, R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.charge(N)?;
        let mut out = [0; N];
        self.input.read_exact(&mut out).map_err(|_| invalid())?;
        Ok(out)
    }
    fn charge(&mut self, n: usize) -> Result<()> {
        self.left = self.left.checked_sub(n).ok_or_else(invalid)?;
        Ok(())
    }
    fn charge_value(&mut self, n: usize) -> Result<()> {
        self.values_left = self.values_left.checked_sub(n).ok_or_else(invalid)?;
        Ok(())
    }
    fn tag(&mut self) -> Result<u8> {
        Ok(self.bytes::<1>()?[0])
    }
    fn count(&mut self) -> Result<usize> {
        let n = usize::try_from(u64::from_le_bytes(self.bytes()?)).map_err(|_| invalid())?;
        if n > self.left {
            return Err(invalid());
        }
        Ok(n)
    }
    fn string(&mut self) -> Result<String> {
        let n = self.count()?;
        self.charge(n)?;
        self.charge_value(n)?;
        let mut bytes = vec![0; n];
        self.input.read_exact(&mut bytes).map_err(|_| invalid())?;
        String::from_utf8(bytes).map_err(|_| invalid())
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        self.charge_value(8)?;
        Ok(match self.tag()? {
            0 => Value::Null,
            1 => Value::Bool(match self.tag()? {
                0 => false,
                1 => true,
                _ => return Err(invalid()),
            }),
            2 => Value::Int(i64::from_le_bytes(self.bytes()?)),
            3 => Value::float(f64::from_le_bytes(self.bytes()?)).ok_or_else(invalid)?,
            4 => Value::Str(self.string()?),
            tag @ (5 | 6) => {
                if depth >= self.depth {
                    return Err(invalid());
                }
                let n = self.count()?;
                if n > self.values_left / 8 {
                    return Err(invalid());
                }
                // No capacity from an untrusted count: charge each element first.
                if tag == 5 {
                    let mut values = Vec::new();
                    for _ in 0..n {
                        values.push(self.value(depth + 1)?);
                    }
                    Value::List(values)
                } else {
                    let mut values = Vec::new();
                    for _ in 0..n {
                        values.push((self.string()?, self.value(depth + 1)?));
                    }
                    Value::Map(Map::new(values).map_err(|_| invalid())?)
                }
            }
            _ => return Err(invalid()),
        })
    }
    fn field(&mut self, field: &crate::Field) -> Result<PreparedField> {
        let state = match self.tag()? {
            0 if !field.kind.is_required() => FieldState::Absent,
            1 if !field.kind.is_required() => FieldState::Null,
            2 => {
                let FieldKind::Text(options) = field.kind else {
                    return Err(invalid());
                };
                self.charge_value(8)?;
                let text = self.string()?;
                let mut normalized = String::new();
                normalize_text(&text, options.case_fold, &mut normalized);
                if normalized != text {
                    return Err(invalid());
                }
                FieldState::Text(text.into_boxed_str())
            }
            3 => {
                let FieldKind::Exact(options) = field.kind else {
                    return Err(invalid());
                };
                let v = self.normalized()?;
                if matches!(v, NormalizedValue::Null) {
                    return Err(invalid());
                }
                if options.normalizer == crate::ExactNormalizer::Hostname {
                    let NormalizedValue::Str(text) = &v else {
                        return Err(invalid());
                    };
                    let mut normalized = String::new();
                    normalize_hostname(text, &mut normalized).map_err(|_| invalid())?;
                    if normalized != text.as_ref() {
                        return Err(invalid());
                    }
                }
                FieldState::Exact(v)
            }
            _ => return Err(invalid()),
        };
        let flags = self.tag()?;
        let allowed = match field.kind {
            FieldKind::Text(text) => {
                if text.case_fold {
                    3
                } else {
                    1
                }
            }
            FieldKind::Exact(options) if options.normalizer == crate::ExactNormalizer::Hostname => {
                7
            }
            FieldKind::Exact(_) => 9,
        };
        if flags & !allowed != 0
            || (matches!(state, FieldState::Absent | FieldState::Null) && flags != 0)
        {
            return Err(invalid());
        }
        let transforms = [
            Transforms::UNICODE_FORM,
            Transforms::CASE,
            Transforms::TRAILING_DOT,
            Transforms::KEY_ORDER,
        ]
        .iter()
        .enumerate()
        .fold(Transforms::NONE, |bits, (i, flag)| {
            if flags & (1 << i) == 0 {
                bits
            } else {
                bits | *flag
            }
        });
        let mut stages = [0; 3];
        for stage in &mut stages {
            *stage = u64::from_le_bytes(self.bytes()?);
        }
        let trigrams = match (&state, &field.kind) {
            (FieldState::Text(text), FieldKind::Text(options))
                if options.mode != TextMode::Semantic =>
            {
                Some(crate::trigrams(text))
            }
            _ => None,
        };
        let acronyms = match &state {
            FieldState::Text(text) => crate::AcronymKeys::of(text),
            _ => crate::AcronymKeys::default(),
        };
        Ok(PreparedField {
            state,
            transforms,
            stages: Stages(stages),
            trigrams,
            acronyms,
        })
    }
    fn normalized(&mut self) -> Result<NormalizedValue> {
        let input = self.value(0)?;
        let (normalized, transforms) = normalize_value(&input).map_err(|_| invalid())?;
        if !transforms.is_empty() {
            return Err(invalid());
        }
        Ok(normalized)
    }
}

/// Reads and validates one record, bounding allocations before copying data.
/// The encoded allowance includes normalization expansion and wire metadata.
/// # Errors
/// Rejects malformed, oversized, non-canonical or schema-incompatible data.
pub fn read_record(
    input: &mut impl Read,
    schema: &Schema,
    max_bytes: usize,
) -> Result<PreparedRecord> {
    let allowance = schema
        .limits()
        .max_selected_bytes
        .saturating_mul(3)
        .saturating_add(
            (schema.fields().len() + schema.partition_by().len() + 1).saturating_mul(40),
        );
    let mut reader = Reader {
        input,
        left: allowance.min(max_bytes),
        depth: schema.limits().max_depth,
        values_left: schema.limits().max_selected_bytes.saturating_mul(3),
    };
    let id = match reader.tag()? {
        0 if schema.id_field().is_none() => None,
        1 if schema.id_field().is_some() => {
            reader.charge_value(8)?;
            Some(RecordId::Int(i64::from_le_bytes(reader.bytes()?)))
        }
        2 if schema.id_field().is_some() => {
            reader.charge_value(8)?;
            Some(RecordId::Str(reader.string()?.into_boxed_str()))
        }
        _ => return Err(invalid()),
    };
    let mut partition = Vec::new();
    for _ in schema.partition_by() {
        let v = reader.normalized()?;
        if matches!(
            v,
            NormalizedValue::Null | NormalizedValue::List(_) | NormalizedValue::Map(_)
        ) {
            return Err(invalid());
        }
        partition.push(v);
    }
    let mut fields = Vec::new();
    for field in schema.fields() {
        let prepared = reader.field(field)?;
        if schema.compares_whole_record() && !matches!(prepared.state, FieldState::Text(_)) {
            return Err(invalid());
        }
        fields.push(prepared);
    }

    Ok(PreparedRecord {
        schema: schema.preparation_fingerprint(),
        id,
        partition: partition.into_boxed_slice(),
        fields: fields.into_boxed_slice(),
    })
}
