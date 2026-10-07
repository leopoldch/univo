use std::fmt;

use crate::path::FieldPath;

/// Errors raised by the core. Bindings map each variant to a stable public
/// exception; variants are added as the engine grows.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The schema is inconsistent; nothing was compared.
    InvalidSchema { reason: String },
    /// A record does not satisfy the schema.
    InvalidRecord { path: FieldPath, reason: String },
    /// Records prepared with different schemas were compared.
    SchemaMismatch,
    /// A record exceeds a configured resource limit. When the check stops
    /// at the first excess, `actual` is the first value above `limit`.
    ResourceLimit {
        path: FieldPath,
        resource: Resource,
        limit: usize,
        actual: usize,
    },
}

/// Resource bounded by [`crate::Limits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    NestingDepth,
    SelectedBytes,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub(crate) fn schema(reason: impl Into<String>) -> Self {
        Self::InvalidSchema {
            reason: reason.into(),
        }
    }

    pub(crate) fn record(path: &FieldPath, reason: impl Into<String>) -> Self {
        Self::InvalidRecord {
            path: path.clone(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { reason } => write!(f, "invalid schema: {reason}"),
            Self::InvalidRecord { path, reason } => write!(f, "invalid record at {path}: {reason}"),
            Self::SchemaMismatch => f.write_str(
                "records were prepared with a schema whose preparation differs from this one",
            ),
            Self::ResourceLimit {
                path,
                resource,
                limit,
                actual,
            } => write!(
                f,
                "{resource} at {path} is {actual}, above the limit of {limit}"
            ),
        }
    }
}

impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NestingDepth => "nesting depth",
            Self::SelectedBytes => "selected size in bytes",
        })
    }
}

impl std::error::Error for Error {}
