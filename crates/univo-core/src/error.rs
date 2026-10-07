//! Errors shared by every binding.
//!
//! An error carries everything a binding needs to raise an idiomatic
//! exception without composing any text: a [`ErrorKind`] (one exception type
//! per kind in each language), a stable [`ErrorCode`] documented in
//! `docs/errors.md`, a complete message, a hint, the field path, structured
//! details and, for batches, the position of the record.
//!
//! Messages name paths, types and sizes but never field values: errors often
//! end up in logs, and record contents must not.

use std::fmt;

use crate::normalize::KeyCollision;
use crate::path::FieldPath;
use crate::prepare::RecordId;

/// Category of an error. Bindings map each kind to one exception type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The schema is inconsistent; nothing was compared.
    InvalidSchema,
    /// A record does not satisfy the schema.
    InvalidRecord,
    /// A record exceeds a configured resource limit.
    ResourceLimit,
    /// Records prepared with different schemas were compared.
    SchemaMismatch,
}

impl ErrorKind {
    /// Name shared by the bindings, which append `Error` or `Exception`.
    pub fn name(self) -> &'static str {
        match self {
            Self::InvalidSchema => "InvalidSchema",
            Self::InvalidRecord => "InvalidRecord",
            Self::ResourceLimit => "ResourceLimit",
            Self::SchemaMismatch => "SchemaMismatch",
        }
    }
}

macro_rules! error_codes {
    ($($variant:ident => $code:literal, $kind:ident, $title:literal, $hint:literal;)+) => {
        /// Precise cause of an error. Codes are stable: a released code keeps
        /// its meaning, and a retired one is never reused.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum ErrorCode {
            $($variant,)+
        }

        impl ErrorCode {
            /// Every code, in documentation order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// Stable identifier, such as `record.wrong_type`.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            pub fn kind(self) -> ErrorKind {
                match self {
                    $(Self::$variant => ErrorKind::$kind,)+
                }
            }

            /// Short description of the cause.
            pub fn title(self) -> &'static str {
                match self {
                    $(Self::$variant => $title,)+
                }
            }

            /// How to fix the cause.
            pub fn hint(self) -> &'static str {
                match self {
                    $(Self::$variant => $hint,)+
                }
            }
        }
    };
}

error_codes! {
    NoFields => "schema.no_fields", InvalidSchema,
        "The schema compares no field",
        "Add at least one Text or Exact field to the schema.";
    TooManyFields => "schema.too_many_fields", InvalidSchema,
        "The schema compares more fields than allowed",
        "Select fewer fields, or raise Limits.max_fields if every field is needed.";
    InvalidWeight => "schema.invalid_weight", InvalidSchema,
        "A field weight is not a positive finite number",
        "Use a finite weight greater than zero, such as 1.0.";
    InvalidLimit => "schema.invalid_limit", InvalidSchema,
        "A resource limit is out of range",
        "Use limits greater than zero; max_depth cannot exceed 128.";
    InvalidPath => "schema.invalid_path", InvalidSchema,
        "A field path is empty or contains an empty key",
        "Write paths as non-empty dotted keys such as \"params.retries\"; build the path from explicit keys when a key contains a dot.";
    OverlappingPaths => "schema.overlapping_paths", InvalidSchema,
        "Two paths of the schema designate the same or nested values",
        "Make the identifier, the partition keys and the compared fields designate distinct values; compare either an object or its members, not both.";
    WholeRecordField => "schema.whole_record_field", InvalidSchema,
        "A field designates the whole record in an object schema",
        "Use Schema::strings() to compare plain strings, or give every field a path.";
    WrongShape => "record.wrong_shape", InvalidRecord,
        "The record, or a value on the way to a field, has the wrong shape",
        "Pass an object (a dict in Python) whose keys follow the schema paths, or a string for a strings schema.";
    WrongType => "record.wrong_type", InvalidRecord,
        "A field holds a value of the wrong type",
        "Convert the value before passing it: Univo never converts types implicitly, so 3 and \"3\" are different values.";
    UnsupportedType => "record.unsupported_type", InvalidRecord,
        "A field holds a value of a type Univo cannot represent",
        "Convert the value to a string, integer, float, boolean, null, list or object.";
    NonFiniteNumber => "record.non_finite_number", InvalidRecord,
        "A field holds NaN or an infinite number",
        "Replace NaN and infinities with a finite number or with null.";
    IntegerOutOfRange => "record.integer_out_of_range", InvalidRecord,
        "A field holds an integer outside the signed 64-bit range",
        "Pass integers beyond 64 bits as strings, and declare the field accordingly.";
    MissingIdentifier => "record.missing_identifier", InvalidRecord,
        "The record has no identifier",
        "Give every record a string or integer identifier at the schema's id path.";
    MissingPartitionKey => "record.missing_partition_key", InvalidRecord,
        "The record has no value for a partition key",
        "Give every record a value for each partition key: records are only compared within one partition.";
    MissingRequiredField => "record.missing_required_field", InvalidRecord,
        "A required field is absent or null",
        "Provide the field, or declare it optional (required=False) if records may lack it.";
    DuplicateKey => "record.duplicate_key", InvalidRecord,
        "An object holds the same key twice",
        "Remove the repeated key.";
    KeyCollision => "record.key_collision", InvalidRecord,
        "Two keys of an object differ only by their Unicode encoding",
        "Keep one of the two keys: after Unicode normalization they are the same key.";
    InvalidHostname => "record.invalid_hostname", InvalidRecord,
        "A hostname ends with more than one dot",
        "Remove the extra trailing dots: a hostname may end with at most one.";
    NestingTooDeep => "limit.nesting_depth", ResourceLimit,
        "A selected value is nested too deeply",
        "Flatten the value, select a nested path instead of the whole object, or raise Limits.max_depth (at most 128).";
    RecordTooLarge => "limit.record_size", ResourceLimit,
        "The selected values of a record are too large",
        "Select fewer or smaller fields, or raise Limits.max_selected_bytes.";
    SchemaMismatch => "state.schema_mismatch", SchemaMismatch,
        "Records prepared with different schemas were compared",
        "Prepare both records with the schema used for the comparison.";
}

/// Base URL of the error catalog; each code has an anchor there.
const CATALOG_URL: &str = "https://github.com/leopoldch/univo/blob/main/docs/errors.md";

impl ErrorCode {
    /// Anchor of the code in `docs/errors.md`, as GitHub derives it from
    /// the heading.
    pub fn anchor(self) -> String {
        self.as_str().replace('.', "")
    }

    pub fn documentation_url(self) -> String {
        format!("{CATALOG_URL}#{}", self.anchor())
    }
}

/// Value of a structured detail.
#[derive(Debug, Clone, PartialEq)]
pub enum Detail {
    Text(String),
    Count(u64),
    Number(f64),
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::Count(count) => write!(f, "{count}"),
            Self::Number(number) => write!(f, "{number}"),
        }
    }
}

/// Position of a record in a batch, set by batch operations.
#[derive(Clone, PartialEq, Eq)]
pub struct RecordLocation {
    /// Zero-based position in the batch.
    pub index: usize,
    /// Identifier, when the record has a valid one. Identifiers can be
    /// personal data: they are never printed, by `Display` or by `Debug`.
    pub id: Option<RecordId>,
}

impl fmt::Debug for RecordLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordLocation")
            .field("index", &self.index)
            .field("id", &self.id.as_ref().map(|_| Redacted))
            .finish()
    }
}

/// Placeholder printed instead of a record identifier.
struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// An error with its code, message, hint and context.
///
/// Boxed, so that `Result<T, Error>` costs one pointer on the success path.
#[derive(Debug, Clone, PartialEq)]
pub struct Error(Box<Inner>);

#[derive(Debug, Clone, PartialEq)]
struct Inner {
    code: ErrorCode,
    message: String,
    path: Option<FieldPath>,
    details: Vec<(&'static str, Detail)>,
    record: Option<RecordLocation>,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    fn new(code: ErrorCode, message: String) -> Self {
        Self(Box::new(Inner {
            code,
            message,
            path: None,
            details: Vec::new(),
            record: None,
        }))
    }

    fn at(mut self, path: &FieldPath) -> Self {
        self.0.path = Some(path.clone());
        self
    }

    fn detail(mut self, key: &'static str, value: Detail) -> Self {
        self.0.details.push((key, value));
        self
    }

    fn text(self, key: &'static str, value: impl Into<String>) -> Self {
        self.detail(key, Detail::Text(value.into()))
    }

    fn count(self, key: &'static str, value: usize) -> Self {
        self.detail(key, Detail::Count(u64::try_from(value).unwrap_or(u64::MAX)))
    }

    /// Locates the error in a batch.
    #[must_use]
    pub fn with_record(mut self, index: usize, id: Option<RecordId>) -> Self {
        self.0.record = Some(RecordLocation { index, id });
        self
    }

    pub fn code(&self) -> ErrorCode {
        self.0.code
    }

    pub fn kind(&self) -> ErrorKind {
        self.0.code.kind()
    }

    /// Complete description of this occurrence, without the hint.
    pub fn message(&self) -> &str {
        &self.0.message
    }

    pub fn hint(&self) -> &'static str {
        self.0.code.hint()
    }

    /// Path of the field concerned, if any.
    pub fn path(&self) -> Option<&FieldPath> {
        self.0.path.as_ref()
    }

    /// Structured context, in a fixed order per code, for programmatic use.
    pub fn details(&self) -> &[(&'static str, Detail)] {
        &self.0.details
    }

    pub fn record(&self) -> Option<&RecordLocation> {
        self.0.record.as_ref()
    }

    pub fn documentation_url(&self) -> String {
        self.0.code.documentation_url()
    }
}

/// `a string`, `an integer`, `null`: type names as they read in a sentence.
fn with_article(type_name: &str) -> String {
    match type_name {
        "null" => "null".to_owned(),
        name if name.starts_with(['a', 'e', 'i', 'o', 'u']) => format!("an {name}"),
        name => format!("a {name}"),
    }
}

fn missing_state(null: bool) -> &'static str {
    if null { "null" } else { "absent" }
}

/// One constructor per code, so that a code always carries the same details.
/// Bindings use the public ones while converting foreign values.
impl Error {
    pub(crate) fn no_fields() -> Self {
        Self::new(
            ErrorCode::NoFields,
            "the schema selects no field to compare".to_owned(),
        )
    }

    pub(crate) fn too_many_fields(count: usize, limit: usize) -> Self {
        Self::new(
            ErrorCode::TooManyFields,
            format!("the schema selects {count} fields, above the limit of {limit}"),
        )
        .count("count", count)
        .count("limit", limit)
    }

    pub(crate) fn invalid_weight(path: &FieldPath, weight: f32) -> Self {
        Self::new(
            ErrorCode::InvalidWeight,
            format!("field `{path}` has weight {weight}, which is not a positive finite number"),
        )
        .at(path)
        .detail("weight", Detail::Number(f64::from(weight)))
    }

    pub(crate) fn invalid_limit(name: &'static str, value: usize, reason: &str) -> Self {
        Self::new(
            ErrorCode::InvalidLimit,
            format!("limit {name} is {value}, but {reason}"),
        )
        .text("limit", name)
        .count("value", value)
    }

    pub(crate) fn invalid_path(written: &str, reason: &str) -> Self {
        Self::new(
            ErrorCode::InvalidPath,
            format!("field path {written:?} is invalid: {reason}"),
        )
        .text("written", written)
    }

    pub(crate) fn overlapping_paths(
        first_role: &'static str,
        first: &FieldPath,
        second_role: &'static str,
        second: &FieldPath,
    ) -> Self {
        let relation = if first == second {
            "is the same as"
        } else {
            "overlaps with"
        };
        Self::new(
            ErrorCode::OverlappingPaths,
            format!("{second_role} `{second}` {relation} {first_role} `{first}`"),
        )
        .at(second)
        .text("first", first.to_string())
        .text("second", second.to_string())
    }

    pub(crate) fn whole_record_field() -> Self {
        Self::new(
            ErrorCode::WholeRecordField,
            "a field designates the whole record in a schema of objects".to_owned(),
        )
    }

    pub(crate) fn wrong_shape(path: &FieldPath, expected: &str, found: &str) -> Self {
        let message = if path.is_root() {
            format!(
                "the record must be {expected}, found {}",
                with_article(found)
            )
        } else {
            format!(
                "`{path}` must be {expected} to reach the fields below it, found {}",
                with_article(found)
            )
        };
        Self::new(ErrorCode::WrongShape, message)
            .at(path)
            .text("expected", expected)
            .text("found", found)
    }

    pub(crate) fn wrong_type(path: &FieldPath, expected: &str, found: &str) -> Self {
        Self::new(
            ErrorCode::WrongType,
            format!(
                "field `{path}` must be {expected}, found {}",
                with_article(found)
            ),
        )
        .at(path)
        .text("expected", expected)
        .text("found", found)
    }

    /// A foreign value of a type with no Univo equivalent, named as the
    /// caller's language names it.
    pub fn unsupported_type(path: &FieldPath, type_name: &str) -> Self {
        Self::new(
            ErrorCode::UnsupportedType,
            format!("field `{path}` holds a value of unsupported type `{type_name}`"),
        )
        .at(path)
        .text("found", type_name)
    }

    pub fn non_finite_number(path: &FieldPath) -> Self {
        Self::new(
            ErrorCode::NonFiniteNumber,
            format!("field `{path}` holds NaN or an infinite number"),
        )
        .at(path)
    }

    pub fn integer_out_of_range(path: &FieldPath) -> Self {
        Self::new(
            ErrorCode::IntegerOutOfRange,
            format!("field `{path}` holds an integer outside the signed 64-bit range"),
        )
        .at(path)
    }

    pub(crate) fn missing_identifier(path: &FieldPath, null: bool) -> Self {
        let state = missing_state(null);
        Self::new(
            ErrorCode::MissingIdentifier,
            format!("the identifier `{path}` is {state}"),
        )
        .at(path)
        .text("state", state)
    }

    pub(crate) fn missing_partition_key(path: &FieldPath, null: bool) -> Self {
        let state = missing_state(null);
        Self::new(
            ErrorCode::MissingPartitionKey,
            format!("the partition key `{path}` is {state}"),
        )
        .at(path)
        .text("state", state)
    }

    pub(crate) fn missing_required_field(path: &FieldPath, null: bool) -> Self {
        let state = missing_state(null);
        Self::new(
            ErrorCode::MissingRequiredField,
            format!("required field `{path}` is {state}"),
        )
        .at(path)
        .text("state", state)
    }

    /// An object built from foreign data holds `key` twice.
    pub fn duplicate_key(path: &FieldPath, key: &str) -> Self {
        Self::new(
            ErrorCode::DuplicateKey,
            format!("the object at `{path}` holds the key {key:?} twice"),
        )
        .at(path)
        .text("key", key)
    }

    pub(crate) fn key_collision(path: &FieldPath, collision: &KeyCollision) -> Self {
        let key = &collision.key;
        let message = if collision.location.is_empty() {
            format!(
                "the object at `{path}` holds two keys that both become {key:?} after Unicode normalization"
            )
        } else {
            format!(
                "field `{path}` contains, at `{}`, an object with two keys that both become {key:?} after Unicode normalization",
                collision.location
            )
        };
        Self::new(ErrorCode::KeyCollision, message)
            .at(path)
            .text("key", &**key)
            .text("location", collision.location.clone())
    }

    pub(crate) fn invalid_hostname(path: &FieldPath) -> Self {
        Self::new(
            ErrorCode::InvalidHostname,
            format!("hostname field `{path}` ends with more than one dot"),
        )
        .at(path)
    }

    pub(crate) fn nesting_too_deep(path: &FieldPath, limit: usize) -> Self {
        Self::new(
            ErrorCode::NestingTooDeep,
            format!("field `{path}` is nested more than {limit} levels deep"),
        )
        .at(path)
        .count("limit", limit)
    }

    pub(crate) fn record_too_large(path: &FieldPath, limit: usize, reached: usize) -> Self {
        Self::new(
            ErrorCode::RecordTooLarge,
            format!(
                "the selected values reach at least {reached} bytes at `{path}`, above the limit of {limit} bytes"
            ),
        )
        .at(path)
        .count("limit", limit)
        .count("reached", reached)
    }

    pub(crate) fn schema_mismatch() -> Self {
        Self::new(
            ErrorCode::SchemaMismatch,
            "the records were prepared with a schema that normalizes differently from the one used to compare them"
                .to_owned(),
        )
    }
}

/// Three lines: what happened, how to fix it, where to read more.
///
/// ```text
/// record 12: field `name` must be a string, found an integer
/// hint: Convert the value before passing it: ...
/// code: record.wrong_type (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordwrong_type)
/// ```
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(record) = &self.0.record {
            write!(f, "record {}: ", record.index)?;
        }
        writeln!(f, "{}", self.0.message)?;
        writeln!(f, "hint: {}", self.hint())?;
        write!(
            f,
            "code: {} ({})",
            self.0.code.as_str(),
            self.documentation_url()
        )
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_stay_one_pointer_wide() {
        assert_eq!(
            std::mem::size_of::<Result<()>>(),
            std::mem::size_of::<usize>()
        );
    }

    #[test]
    fn codes_are_unique_namespaced_and_complete() {
        let mut seen = std::collections::HashSet::new();
        for code in ErrorCode::ALL {
            assert!(
                seen.insert(code.as_str()),
                "{} is duplicated",
                code.as_str()
            );
            let (namespace, name) = code.as_str().split_once('.').unwrap();
            assert!(["schema", "record", "limit", "state"].contains(&namespace));
            assert!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            );
            assert!(code.title().len() > 10, "{}", code.as_str());
            assert!(code.hint().ends_with('.'), "{}", code.as_str());
        }
    }

    #[test]
    fn display_has_message_hint_and_code() {
        let path = FieldPath::parse("params.retries").unwrap();
        let error = Error::wrong_type(&path, "a string", "integer").with_record(12, None);
        let text = error.to_string();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(
            lines[0],
            "record 12: field `params.retries` must be a string, found an integer"
        );
        assert!(lines[1].starts_with("hint: Convert the value"));
        assert!(lines[2].starts_with("code: record.wrong_type (https://"));
        assert!(lines[2].ends_with("errors.md#recordwrong_type)"));
    }

    #[test]
    fn articles_read_naturally() {
        assert_eq!(with_article("integer"), "an integer");
        assert_eq!(with_article("object"), "an object");
        assert_eq!(with_article("string"), "a string");
        assert_eq!(with_article("null"), "null");
    }
}
