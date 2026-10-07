//! Normalization of compared values, as defined in
//! `docs/duplicate-definitions.md`.
//!
//! Every function reports which [`Transforms`] changed its input, so that an
//! exact match can explain what it ignored.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::ops::{BitOr, BitOrAssign};
use std::sync::OnceLock;

use caseless::Caseless;
use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};

use crate::value::{Finite, Value};

/// Version of the normalization rules. Any change in output for some input
/// must increment it, so that caches and indexes built with another version
/// are rejected instead of mixed.
pub const NORMALIZER_VERSION: u32 = 1;

/// Set of normalization steps that changed a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Transforms(u8);

impl Transforms {
    pub const NONE: Self = Self(0);
    /// Unicode NFC composition.
    pub const UNICODE_FORM: Self = Self(1);
    /// Case folding: full Unicode for text, ASCII only for hostnames.
    pub const CASE: Self = Self(1 << 1);
    /// Removal of a trailing dot from a hostname.
    pub const TRAILING_DOT: Self = Self(1 << 2);
    /// Reordering of object keys.
    pub const KEY_ORDER: Self = Self(1 << 3);

    const NAMES: [(Self, &'static str); 4] = [
        (Self::UNICODE_FORM, "unicode form"),
        (Self::CASE, "case"),
        (Self::TRAILING_DOT, "trailing dot"),
        (Self::KEY_ORDER, "key order"),
    ];

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Human-readable names of the steps, in a fixed order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMES
            .into_iter()
            .filter_map(move |(flag, name)| self.contains(flag).then_some(name))
    }
}

impl BitOr for Transforms {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for Transforms {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl fmt::Display for Transforms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, name) in self.names().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            f.write_str(name)?;
        }
        Ok(())
    }
}

/// Every character below U+0300 is in NFC whatever its neighbours, because
/// canonical composition always involves a character at or above U+0300.
/// In UTF-8 those characters are exactly the ones whose bytes are all below
/// `0xCC`, which turns the common Latin case into a plain byte scan.
fn below_combining_marks(text: &str) -> bool {
    // A full reduction, unlike a short-circuiting scan, vectorizes.
    text.is_ascii() || text.bytes().fold(0, u8::max) < 0xCC
}

fn is_nfc(text: &str) -> bool {
    below_combining_marks(text) || is_nfc_quick(text.chars()) == IsNormalized::Yes
}

/// Appends the NFC form of `input` to `out`; returns true if it differs.
///
/// Only segments around characters at or above U+0300 are composed: every
/// character below U+0300 is a starter in NFC and never the second element
/// of a composition, so composition cannot cross it.
pub(crate) fn push_nfc(input: &str, out: &mut String) -> bool {
    if is_nfc(input) {
        out.push_str(input);
        return false;
    }
    let start = out.len();
    let mut copied = 0;
    while let Some(relative) = input.as_bytes()[copied..]
        .iter()
        .position(|&byte| byte >= 0xCC)
    {
        let first_high = copied + relative;
        let segment_start = input[copied..first_high]
            .char_indices()
            .next_back()
            .map_or(first_high, |(at, _)| copied + at);
        let segment_end = input[first_high..]
            .char_indices()
            .find(|&(_, character)| u32::from(character) < 0x300)
            .map_or(input.len(), |(at, _)| first_high + at);
        out.push_str(&input[copied..segment_start]);
        let segment = &input[segment_start..segment_end];
        if is_nfc_quick(segment.chars()) == IsNormalized::Yes {
            out.push_str(segment);
        } else {
            out.extend(segment.nfc());
        }
        copied = segment_end;
    }
    out.push_str(&input[copied..]);
    out[start..] != *input
}

/// Puts `out[start..]` back in NFC after a step that may have decomposed a
/// character or placed a new base before a combining mark.
fn recompose_from(out: &mut String, start: usize) {
    if !is_nfc(&out[start..]) {
        let tail = out.split_off(start);
        push_nfc(&tail, out);
    }
}

/// Single-character case folding of every code point below U+0300, or NUL
/// when the folding expands to several characters (`ß` to `ss`). Derived
/// once from the same Unicode data as the general path.
fn latin_folds() -> &'static [char] {
    static TABLE: OnceLock<Box<[char]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..0x300u32)
            .map(|code| {
                let character = char::from_u32(code).unwrap_or('\0');
                let mut folded = std::iter::once(character).default_case_fold();
                match (folded.next(), folded.next()) {
                    (Some(single), None) => single,
                    _ => '\0',
                }
            })
            .collect()
    })
}

/// Ranges without any case folding: CJK, kana, Yi and Hangul syllables.
fn has_no_case_folding(character: char) -> bool {
    matches!(u32::from(character), 0x3040..0xA640 | 0xAC00..0xD7A4)
}

/// Single-character case folding of `character`, or NUL when it expands to
/// several characters.
fn single_fold(character: char) -> char {
    let latin = latin_folds();
    let code = u32::from(character) as usize;
    if code < latin.len() {
        return latin[code];
    }
    if has_no_case_folding(character) {
        return character;
    }
    let mut folded = std::iter::once(character).default_case_fold();
    match (folded.next(), folded.next()) {
        (Some(single), None) => single,
        _ => '\0',
    }
}

/// Appends the full case folding of `input` to `out`.
///
/// The text is copied and its ASCII lowercased in one vectorized pass, then
/// only non-ASCII characters are visited. Text whose non-ASCII characters
/// already fold to themselves, such as lowercase accented letters, needs no
/// further work; from the first one that changes, the rest is rebuilt run by
/// run.
fn push_case_folded(input: &str, out: &mut String) {
    let start = out.len();
    out.push_str(input);
    out[start..].make_ascii_lowercase();

    let mut offset = 0;
    while let Some(at) = next_non_ascii(input, offset) {
        let character = input[at..].chars().next().unwrap_or_default();
        offset = at + character.len_utf8();
        if single_fold(character) != character {
            let mut bytes = std::mem::take(out).into_bytes();
            bytes.truncate(start + at);
            push_case_folded_runs(&input[at..], &mut bytes);
            *out = into_string(bytes);
            return;
        }
    }
}

/// Appends the case folding of `input` run by run: each ASCII run is copied
/// and lowercased as a block, each other character folded on its own.
fn push_case_folded_runs(input: &str, bytes: &mut Vec<u8>) {
    let mut offset = 0;
    loop {
        let at = next_non_ascii(input, offset).unwrap_or(input.len());
        let run_start = bytes.len();
        bytes.extend_from_slice(&input.as_bytes()[offset..at]);
        bytes[run_start..].make_ascii_lowercase();
        let Some(character) = input[at..].chars().next() else {
            return;
        };
        offset = at + character.len_utf8();
        let mut buffer = [0; 4];
        match single_fold(character) {
            '\0' => {
                for folded in std::iter::once(character).default_case_fold() {
                    bytes.extend_from_slice(folded.encode_utf8(&mut buffer).as_bytes());
                }
            }
            folded => bytes.extend_from_slice(folded.encode_utf8(&mut buffer).as_bytes()),
        }
    }
}

fn next_non_ascii(input: &str, from: usize) -> Option<usize> {
    input.as_bytes()[from..]
        .iter()
        .position(|byte| !byte.is_ascii())
        .map(|relative| from + relative)
}

fn into_string(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|error| {
        // Unreachable: only whole UTF-8 sequences are written.
        String::from_utf8_lossy(error.as_bytes()).into_owned()
    })
}

/// Normalizes free text into `out`: NFC, then optionally Unicode full case
/// folding followed by NFC again.
pub fn normalize_text(input: &str, case_fold: bool, out: &mut String) -> Transforms {
    normalize_text_observed(input, case_fold, out, |_| {})
}

/// [`normalize_text`], also passing the intermediate NFC form to `composed`
/// when it differs from `input`, so that callers need not recompute it.
pub(crate) fn normalize_text_observed(
    input: &str,
    case_fold: bool,
    out: &mut String,
    composed_form: impl FnOnce(&str),
) -> Transforms {
    let start = out.len();
    if input.is_ascii() {
        out.push_str(input);
        if case_fold && input.bytes().any(|byte| byte.is_ascii_uppercase()) {
            out[start..].make_ascii_lowercase();
            return Transforms::CASE;
        }
        return Transforms::NONE;
    }

    let mut transforms = Transforms::NONE;
    let composed: Cow<'_, str> = if is_nfc(input) {
        Cow::Borrowed(input)
    } else {
        let mut composed = String::with_capacity(input.len());
        if push_nfc(input, &mut composed) {
            transforms |= Transforms::UNICODE_FORM;
            composed_form(&composed);
        }
        Cow::Owned(composed)
    };
    if !case_fold {
        out.push_str(&composed);
        return transforms;
    }
    push_case_folded(&composed, out);
    if out[start..] != *composed {
        recompose_from(out, start);
        if out[start..] != *composed {
            transforms |= Transforms::CASE;
        }
    }
    transforms
}

/// Normalizes a hostname into `out`: NFC, ASCII-only lowercasing, NFC again,
/// and removal of one trailing dot. Non-ASCII characters are kept, because
/// full case folding would merge distinct IDNA2008 names such as `faß.de`
/// and `fass.de`. The second NFC composes a lowered letter with a following
/// combining mark (`T` + U+0308 becomes U+1E97), which keeps the function
/// idempotent.
pub fn normalize_hostname(input: &str, out: &mut String) -> Transforms {
    let start = out.len();
    let mut transforms = Transforms::NONE;
    if push_nfc(input, out) {
        transforms |= Transforms::UNICODE_FORM;
    }
    if out[start..].bytes().any(|byte| byte.is_ascii_uppercase()) {
        out[start..].make_ascii_lowercase();
        recompose_from(out, start);
        transforms |= Transforms::CASE;
    }
    if out.len() > start && out.ends_with('.') {
        out.pop();
        transforms |= Transforms::TRAILING_DOT;
    }
    transforms
}

/// Canonical form of a value compared for typed equality: strings and keys
/// in NFC, object keys sorted, list order kept.
#[derive(Debug, Clone, PartialEq)]
pub enum NormalizedValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(Finite),
    Str(Box<str>),
    List(Box<[NormalizedValue]>),
    Map(Box<[(Box<str>, NormalizedValue)]>),
}

/// Two keys of one object became equal after NFC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCollision(pub Box<str>);

/// Normalizes a value for typed equality.
///
/// # Errors
///
/// Returns [`KeyCollision`] when two keys of one object become equal after
/// NFC, since the object would then be ambiguous.
pub fn normalize_value(value: &Value) -> Result<(NormalizedValue, Transforms), KeyCollision> {
    let mut transforms = Transforms::NONE;
    let normalized = normalize_value_into(value, &mut transforms)?;
    Ok((normalized, transforms))
}

fn normalize_value_into(
    value: &Value,
    transforms: &mut Transforms,
) -> Result<NormalizedValue, KeyCollision> {
    Ok(match value {
        Value::Null => NormalizedValue::Null,
        Value::Bool(flag) => NormalizedValue::Bool(*flag),
        Value::Int(number) => NormalizedValue::Int(*number),
        Value::Float(number) => NormalizedValue::Float(*number),
        Value::Str(text) => NormalizedValue::Str(nfc_boxed(text, transforms)),
        Value::List(items) => NormalizedValue::List(
            items
                .iter()
                .map(|item| normalize_value_into(item, transforms))
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(map) => {
            let mut entries = map
                .entries()
                .iter()
                .map(|(key, item)| {
                    Ok((
                        nfc_boxed(key, transforms),
                        normalize_value_into(item, transforms)?,
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if !entries.is_sorted_by(|left, right| left.0 <= right.0) {
                entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
                *transforms |= Transforms::KEY_ORDER;
            }
            if let Some(pair) = entries
                .windows(2)
                .find(|pair| pair[0].0.cmp(&pair[1].0) == Ordering::Equal)
            {
                return Err(KeyCollision(pair[0].0.clone()));
            }
            NormalizedValue::Map(entries.into_boxed_slice())
        }
    })
}

fn nfc_boxed(text: &str, transforms: &mut Transforms) -> Box<str> {
    let mut out = String::with_capacity(text.len());
    if push_nfc(text, &mut out) {
        *transforms |= Transforms::UNICODE_FORM;
    }
    out.into_boxed_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Map;

    fn text(input: &str, case_fold: bool) -> (String, Transforms) {
        let mut out = String::new();
        let transforms = normalize_text(input, case_fold, &mut out);
        (out, transforms)
    }

    fn hostname(input: &str) -> (String, Transforms) {
        let mut out = String::new();
        let transforms = normalize_hostname(input, &mut out);
        (out, transforms)
    }

    #[test]
    fn ascii_text_is_lowercased_only_when_folding() {
        assert_eq!(text("IBM", true), ("ibm".into(), Transforms::CASE));
        assert_eq!(text("ibm", true), ("ibm".into(), Transforms::NONE));
        assert_eq!(text("IBM", false), ("IBM".into(), Transforms::NONE));
    }

    #[test]
    fn text_uses_full_case_folding_and_nfc() {
        assert_eq!(text("Straße", true).0, "strasse");
        assert_eq!(text("\u{fb01}nance", true).0, "finance");
        assert_eq!(
            text("cafe\u{301}", true),
            ("café".into(), Transforms::UNICODE_FORM)
        );
        assert_eq!(
            text("CAFE\u{301}", true),
            ("café".into(), Transforms::UNICODE_FORM | Transforms::CASE)
        );
        assert_eq!(text("Cafe\u{301}", false).0, "Café");
    }

    #[test]
    fn text_keeps_accents_width_and_digits() {
        assert_eq!(text("Société 3", true).0, "société 3");
        assert_eq!(text("\u{ff21}BC", true).0, "\u{ff41}bc");
    }

    #[test]
    fn appends_after_existing_content() {
        let mut out = String::from("prefix:");
        normalize_text("ÉTÉ", true, &mut out);
        assert_eq!(out, "prefix:été");
    }

    #[test]
    fn hostname_folds_ascii_only() {
        assert_eq!(
            hostname("Stripe.COM."),
            (
                "stripe.com".into(),
                Transforms::CASE | Transforms::TRAILING_DOT
            )
        );
        assert_eq!(hostname("faß.de").0, "faß.de");
        assert_ne!(hostname("faß.de").0, hostname("fass.de").0);
        assert_eq!(hostname("BÜCHER.de").0, "bÜcher.de");
        assert_eq!(hostname("").0, "");
        assert_eq!(hostname("a..").0, "a.");
    }

    #[test]
    fn lowering_recomposes_with_following_marks() {
        assert_eq!(hostname("T\u{308}.de").0, "\u{1e97}.de");
        assert_eq!(hostname("E\u{301}.de").0, "\u{c9}.de");
        let (folded, transforms) = text("\u{1e97}", true);
        assert_eq!(folded, "\u{1e97}");
        assert_eq!(transforms, Transforms::NONE);
    }

    #[test]
    fn values_keep_types_and_list_order() {
        let (three, _) = normalize_value(&Value::Int(3)).unwrap();
        let (three_text, _) = normalize_value(&Value::Str("3".into())).unwrap();
        assert_ne!(three, three_text);

        let list = |items: [i64; 2]| Value::List(items.map(Value::Int).to_vec());
        assert_ne!(
            normalize_value(&list([1, 2])).unwrap().0,
            normalize_value(&list([2, 1])).unwrap().0
        );
    }

    #[test]
    fn object_key_order_is_ignored_and_reported() {
        let object = |keys: [&str; 2]| {
            Value::Map(Map::new(keys.map(|key| (key.to_owned(), Value::Int(1))).to_vec()).unwrap())
        };
        let (sorted, sorted_transforms) = normalize_value(&object(["a", "b"])).unwrap();
        let (reordered, reordered_transforms) = normalize_value(&object(["b", "a"])).unwrap();
        assert_eq!(sorted, reordered);
        assert_eq!(sorted_transforms, Transforms::NONE);
        assert_eq!(reordered_transforms, Transforms::KEY_ORDER);
    }

    #[test]
    fn keys_equal_after_nfc_collide() {
        let map = Map::new(vec![
            ("caf\u{e9}".to_owned(), Value::Null),
            ("cafe\u{301}".to_owned(), Value::Null),
        ])
        .unwrap();
        assert!(normalize_value(&Value::Map(map)).is_err());
    }

    #[test]
    fn transforms_display_in_fixed_order() {
        let transforms = Transforms::KEY_ORDER | Transforms::CASE;
        assert_eq!(transforms.to_string(), "case, key order");
    }
}
