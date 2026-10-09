//! Local indexing and search for Univo.
//!
//! - [`exact`]: equality after normalization, verified on every hit.
//! - [`lexical`]: records whose text is lexically similar to a query,
//!   through an inverted index of trigrams.
//! - [`vectors`]: embeddings of the semantic text fields.
//! - [`store`]: the append-only records an index covers.
//! - [`search`]: records scoring above a minimum, by the union of one
//!   candidate lane per field, with the exhaustive oracle.
//! - [`dense`]: exhaustive nearest-neighbour search over those embeddings,
//!   the reference for any approximate index.

pub mod dense;
pub mod exact;
pub mod lexical;
pub mod search;
pub mod store;
mod values;
pub mod vectors;

pub use dense::{Neighbor, top_k, within};
pub use exact::{ExactHit, ExactIndex, ExactSearch};
pub use lexical::{LexicalIndex, LexicalMatch, LexicalSearch};
pub use search::{LaneCounts, SearchIndex, SearchMatch, SearchOptions, SearchResult};
pub use store::RecordStore;
pub use vectors::{FieldVectors, RecordVectors, embed_records, is_embedded};
