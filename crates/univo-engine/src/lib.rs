//! Local indexing and search for Univo.
//!
//! - [`exact`]: equality after normalization, verified on every hit.
//! - [`vectors`]: embeddings of the semantic text fields.
//! - [`dense`]: exhaustive nearest-neighbour search over those embeddings,
//!   the reference for any approximate index.

pub mod dense;
pub mod exact;
pub mod vectors;

pub use dense::{Neighbor, top_k, within};
pub use exact::{ExactHit, ExactIndex, ExactSearch};
pub use vectors::{FieldVectors, RecordVectors, embed_records, is_embedded};
