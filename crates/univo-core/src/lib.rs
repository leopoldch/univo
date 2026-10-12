//! Core types and matching logic for Univo.
//!
//! This crate has no dependency on Python, on network clients or on an
//! inference runtime. Bindings and the engine build on top of it.

mod budget;
mod compare;
mod error;
mod exact;
mod fingerprint;
mod lexical;
mod normalize;
mod path;
mod prepare;
mod schema;
mod semantic;
pub mod snapshot;
mod value;

pub use compare::{
    Comparison, FieldComparison, SCORING_VERSION, Signal, compare, compare_with_vectors, has_value,
    lexical_similarity,
};
pub use error::{Detail, Error, ErrorCode, ErrorKind, RecordLocation, Result};
pub use exact::{
    ExactMatch, exact_fingerprint, exact_match, has_exact_evidence, is_exact_match,
    partition_fingerprint, value_fingerprint,
};
pub use lexical::{MAX_EDIT_CHARS, edit_similarity, jaccard, positional_trigrams, trigrams};
pub use normalize::{
    InvalidHostname, KeyCollision, NORMALIZER_VERSION, NormalizedValue, Transforms,
    normalize_hostname, normalize_text, normalize_value,
};
pub use path::FieldPath;
pub use prepare::{FieldState, PreparedField, PreparedRecord, RecordId, prepare};
pub use schema::{
    Conflict, Exact, ExactNormalizer, Field, FieldKind, Limits, MAX_NESTING_DEPTH, Schema,
    SchemaBuilder, Text, TextMode,
};
pub use semantic::{dot, semantic_similarity};
pub use value::{DuplicateKey, Finite, Lookup, Map, Value};

/// Version of the Univo core, shared by every binding.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
