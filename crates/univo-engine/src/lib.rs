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
mod blocks;
pub use ann::AnnOptions;
pub mod dense;
pub mod embedding_cache;
pub mod exact;
pub mod groups;
pub mod lexical;
pub mod search;
pub mod store;
mod values;
pub mod vectors;

pub use dense::{Neighbor, top_k, within};
pub use embedding_cache::{CacheStats, EmbeddingCache};
pub use exact::{ExactHit, ExactIndex, ExactSearch};
pub use groups::{Group, GroupOptions, GroupReport};
pub use lexical::{LexicalIndex, LexicalMatch, LexicalSearch};
pub use search::{
    CheckOrAdd, Decision, LaneCounts, QueryEncoder, SEARCH_VERSION, SearchIndex, SearchMatch,
    SearchOptions, SearchResult,
};
pub use store::RecordStore;
pub use vectors::{FieldVectors, RecordVectors, embed_records, embed_records_cached, is_embedded};

mod report;
pub use report::{PairMatch, PairReport};

/// Folds the queries of an all-pairs report. From a worker of a Rayon pool
/// (inside `ThreadPool::install`) they run in parallel on that pool; from
/// any other thread, in order on it. The caller chooses the threads, and
/// the global pool is never used implicitly.
pub(crate) fn fold_queries<T: Send, A: Send>(
    queries: Vec<T>,
    init: impl Fn() -> A + Sync + Send,
    fold: impl Fn(A, T) -> univo_core::Result<A> + Sync + Send,
    merge: impl Fn(A, A) -> A + Sync + Send,
) -> univo_core::Result<A> {
    use rayon::prelude::*;
    if rayon::current_thread_index().is_none() {
        return queries.into_iter().try_fold(init(), fold);
    }
    queries
        .into_par_iter()
        .try_fold(&init, &fold)
        .try_reduce(&init, |a, b| Ok(merge(a, b)))
}

pub mod snapshot;
