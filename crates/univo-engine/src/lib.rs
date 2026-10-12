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

pub mod ann;
pub use ann::AnnOptions;
pub mod dense;
pub mod embedding_cache;
pub mod exact;
pub mod lexical;
pub mod search;
pub mod store;
mod values;
pub mod vectors;

pub use dense::{Neighbor, top_k, within};
pub use embedding_cache::{CacheStats, EmbeddingCache};
pub use exact::{ExactHit, ExactIndex, ExactSearch};
pub use lexical::{LexicalIndex, LexicalMatch, LexicalSearch};
pub use search::{
    CheckOrAdd, Decision, LaneCounts, QueryEncoder, SEARCH_VERSION, SearchIndex, SearchMatch,
    SearchOptions, SearchResult,
};
pub use store::RecordStore;
pub use vectors::{FieldVectors, RecordVectors, embed_records, embed_records_cached, is_embedded};

mod report;
pub use report::{PairMatch, PairReport};

pub mod snapshot;
