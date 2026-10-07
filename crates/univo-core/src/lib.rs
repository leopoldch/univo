//! Core types and matching logic for Univo.
//!
//! This crate has no dependency on Python, on network clients or on an
//! inference runtime. Bindings and the engine build on top of it.

mod budget;
mod error;
mod exact;
mod fingerprint;
mod normalize;
mod path;
mod prepare;
mod schema;
mod value;

pub use error::{Error, Resource, Result};
pub use exact::{ExactMatch, exact_match};
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
pub use value::{Finite, Lookup, Map, Value};

/// Version of the Univo core, shared by every binding.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
