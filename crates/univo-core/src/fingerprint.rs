//! 64-bit fingerprints of a value at each normalization step.
//!
//! They only explain exact matches: a collision can make an explanation
//! incomplete, never a match wrong, since equality is always decided on the
//! normalized values themselves. Strings, values and normalized values that
//! hold the same content in the same order produce the same fingerprint.

use std::hash::Hasher;

use rustc_hash::FxHasher;

use crate::normalize::{NormalizedValue, Transforms, push_nfc};
use crate::schema::{ExactNormalizer, FieldKind};
use crate::value::Value;

const NULL: u8 = 0;
const BOOL: u8 = 1;
const INT: u8 = 2;
const FLOAT: u8 = 3;
const STR: u8 = 4;
const LIST: u8 = 5;
const MAP: u8 = 6;

/// Fingerprints of a value before each normalization step that changed it.
///
/// Slot `i` holds the fingerprint before `steps(kind)[i]` and is meaningful
/// only when that step is in the field's transforms; the stage after the
/// last step is the normalized value itself and is fingerprinted on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Stages(pub(crate) [u64; 3]);

/// Fingerprints of every stage, from the original value to the normalized
/// one; slots past the last step repeat the normalized fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Trace(pub(crate) [u64; 4]);

impl Trace {
    /// Rebuilds the stages skipped because their step changed nothing.
    pub(crate) fn complete(
        steps: &[Transforms],
        transforms: Transforms,
        stages: Stages,
        normalized: u64,
    ) -> Self {
        let mut full = [normalized; 4];
        for (index, step) in steps.iter().enumerate().rev() {
            full[index] = if transforms.contains(*step) {
                stages.0[index]
            } else {
                full[index + 1]
            };
        }
        Self(full)
    }
}

/// Normalization steps of a field kind, in application order.
pub(crate) fn steps(kind: &FieldKind) -> &'static [Transforms] {
    match kind {
        FieldKind::Text(_) => &[Transforms::UNICODE_FORM, Transforms::CASE],
        FieldKind::Exact(exact) if exact.normalizer == ExactNormalizer::Hostname => &[
            Transforms::UNICODE_FORM,
            Transforms::CASE,
            Transforms::TRAILING_DOT,
        ],
        FieldKind::Exact(_) => &[Transforms::UNICODE_FORM, Transforms::KEY_ORDER],
    }
}

pub(crate) fn text(input: &str) -> u64 {
    let mut hasher = FxHasher::default();
    write_str(&mut hasher, input);
    hasher.finish()
}

/// Fingerprint of a value as given, or with every string in NFC.
pub(crate) fn value(input: &Value, nfc: bool) -> u64 {
    let mut hasher = FxHasher::default();
    write_value(&mut hasher, input, nfc);
    hasher.finish()
}

pub(crate) fn normalized(input: &NormalizedValue) -> u64 {
    let mut hasher = FxHasher::default();
    write_normalized(&mut hasher, input);
    hasher.finish()
}

fn write_str(hasher: &mut FxHasher, text: &str) {
    hasher.write_usize(text.len());
    hasher.write(text.as_bytes());
}

fn write_maybe_nfc(hasher: &mut FxHasher, text: &str, nfc: bool) {
    if nfc && !text.is_ascii() {
        let mut composed = String::with_capacity(text.len());
        push_nfc(text, &mut composed);
        write_str(hasher, &composed);
    } else {
        write_str(hasher, text);
    }
}

fn write_float(hasher: &mut FxHasher, number: f64) {
    hasher.write_u8(FLOAT);
    hasher.write_u64(if number == 0.0 { 0 } else { number.to_bits() });
}

fn write_value(hasher: &mut FxHasher, input: &Value, nfc: bool) {
    match input {
        Value::Null => hasher.write_u8(NULL),
        Value::Bool(flag) => {
            hasher.write_u8(BOOL);
            hasher.write_u8(u8::from(*flag));
        }
        Value::Int(number) => {
            hasher.write_u8(INT);
            hasher.write_i64(*number);
        }
        Value::Float(number) => write_float(hasher, number.get()),
        Value::Str(text) => {
            hasher.write_u8(STR);
            write_maybe_nfc(hasher, text, nfc);
        }
        Value::List(items) => {
            hasher.write_u8(LIST);
            hasher.write_usize(items.len());
            for item in items {
                write_value(hasher, item, nfc);
            }
        }
        Value::Map(map) => {
            hasher.write_u8(MAP);
            hasher.write_usize(map.len());
            for (key, item) in map.entries() {
                write_maybe_nfc(hasher, key, nfc);
                write_value(hasher, item, nfc);
            }
        }
    }
}

fn write_normalized(hasher: &mut FxHasher, input: &NormalizedValue) {
    match input {
        NormalizedValue::Null => hasher.write_u8(NULL),
        NormalizedValue::Bool(flag) => {
            hasher.write_u8(BOOL);
            hasher.write_u8(u8::from(*flag));
        }
        NormalizedValue::Int(number) => {
            hasher.write_u8(INT);
            hasher.write_i64(*number);
        }
        NormalizedValue::Float(number) => write_float(hasher, number.get()),
        NormalizedValue::Str(text) => {
            hasher.write_u8(STR);
            write_str(hasher, text);
        }
        NormalizedValue::List(items) => {
            hasher.write_u8(LIST);
            hasher.write_usize(items.len());
            for item in items {
                write_normalized(hasher, item);
            }
        }
        NormalizedValue::Map(entries) => {
            hasher.write_u8(MAP);
            hasher.write_usize(entries.len());
            for (key, item) in entries {
                write_str(hasher, key);
                write_normalized(hasher, item);
            }
        }
    }
}

/// Steps needed to make two traces equal: the steps applied, on either
/// side, up to the first stage at which their fingerprints agree.
pub(crate) fn needed_steps(steps: &[Transforms], left: Trace, right: Trace) -> Transforms {
    let (left, right) = (left.0, right.0);
    let mut needed = Transforms::NONE;
    for stage in 0..=steps.len() {
        if stage > 0 && (left[stage] != left[stage - 1] || right[stage] != right[stage - 1]) {
            needed |= steps[stage - 1];
        }
        if left[stage] == right[stage] {
            return needed;
        }
    }
    needed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Map;

    #[test]
    fn same_content_same_fingerprint_across_representations() {
        let original = Value::Map(
            Map::new(vec![
                ("a".into(), Value::Str("x".into())),
                ("b".into(), Value::List(vec![Value::Int(1), Value::Null])),
            ])
            .unwrap(),
        );
        let (normalized_value, _) = crate::normalize::normalize_value(&original).unwrap();
        assert_eq!(value(&original, false), normalized(&normalized_value));
        assert_ne!(
            value(&Value::Int(3), false),
            value(&Value::Str("3".into()), false)
        );
    }

    #[test]
    fn nfc_stage_composes_strings() {
        let decomposed = Value::Str("cafe\u{301}".into());
        let composed = Value::Str("caf\u{e9}".into());
        assert_ne!(value(&decomposed, false), value(&composed, false));
        assert_eq!(value(&decomposed, true), value(&composed, false));
    }

    #[test]
    fn complete_fills_unchanged_steps_from_the_next_stage() {
        let steps = [Transforms::UNICODE_FORM, Transforms::CASE];
        let trace = Trace::complete(&steps, Transforms::CASE, Stages([0, 7, 0]), 9);
        assert_eq!(trace, Trace([7, 7, 9, 9]));
        let unchanged = Trace::complete(&steps, Transforms::NONE, Stages::default(), 9);
        assert_eq!(unchanged, Trace([9, 9, 9, 9]));
    }

    #[test]
    fn needed_steps_stop_at_first_equal_stage() {
        let steps = [Transforms::UNICODE_FORM, Transforms::CASE];
        // Both sides change case, but they already agree after NFC.
        let left = Trace([1, 2, 3, 3]);
        let right = Trace([4, 2, 3, 3]);
        assert_eq!(needed_steps(&steps, left, right), Transforms::UNICODE_FORM);
        // They only agree after case folding.
        let left = Trace([1, 1, 3, 3]);
        let right = Trace([2, 2, 3, 3]);
        assert_eq!(needed_steps(&steps, left, right), Transforms::CASE);
        // Identical originals need nothing.
        assert_eq!(
            needed_steps(&steps, Trace([1, 1, 3, 3]), Trace([1, 1, 3, 3])),
            Transforms::NONE
        );
    }
}
