//! Encoding outside a writer's lock, and `check_or_add`: one search and,
//! when nothing matches, one insertion, under the same exclusive borrow.
//!
//! A caller that guards a [`SearchIndex`] with a lock encodes the query
//! with a [`QueryEncoder`] taken under a short shared lock, then calls
//! [`SearchIndex::check_or_add`] under the exclusive lock: the decision and
//! the insertion see the same records. This is local to one index in one
//! process; it does not order writes across processes or databases.

use std::sync::Arc;

use univo_core::{Error, PreparedRecord, Result, Schema};
use univo_embed::Embedder;

use super::{Query, SearchIndex, SearchOptions, SearchResult, check_options};
use crate::EmbeddingCache;
use crate::vectors::{RecordVectors, embed_records_cached};

/// Encodes queries for one index without borrowing it, so that the model
/// can run while other callers search or write.
#[derive(Clone)]
pub struct QueryEncoder {
    schema: Schema,
    embedder: Option<Arc<dyn Embedder>>,
    cache: Option<Arc<EmbeddingCache>>,
}

impl std::fmt::Debug for QueryEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryEncoder")
            .field(
                "model",
                &self
                    .embedder
                    .as_ref()
                    .map(|model| model.descriptor().name()),
            )
            .finish_non_exhaustive()
    }
}

impl QueryEncoder {
    /// The embeddings of `record`'s semantic fields, or `None` when the
    /// index has none.
    ///
    /// # Errors
    ///
    /// Returns `state.schema_mismatch` for a record of another schema and
    /// the embedder's errors.
    pub fn encode(&self, record: &PreparedRecord) -> Result<Option<RecordVectors>> {
        self.encode_batch(std::slice::from_ref(record))
    }

    /// [`QueryEncoder::encode`] for a batch, in one call per semantic field,
    /// for [`SearchIndex::add_encoded`].
    ///
    /// # Errors
    ///
    /// As [`QueryEncoder::encode`].
    pub fn encode_batch(&self, records: &[PreparedRecord]) -> Result<Option<RecordVectors>> {
        if !records
            .iter()
            .all(|record| record.is_prepared_with(&self.schema))
        {
            return Err(Error::schema_mismatch());
        }
        let Some(model) = &self.embedder else {
            return Ok(None);
        };
        embed_records_cached(&self.schema, records, model.as_ref(), self.cache.as_deref()).map(Some)
    }
}

/// What [`SearchIndex::check_or_add`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// No record reached the minimum score; the record was inserted.
    Inserted,
    /// A record equals it on every selected field after normalization.
    Matched,
    /// Records reach the minimum score without being equal: the
    /// application's policy decides.
    Review,
}

/// The decision, the inserted slot, and the search it was based on.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckOrAdd {
    pub decision: Decision,
    /// The new record's slot, when inserted.
    pub slot: Option<usize>,
    /// Matches found before deciding: empty when inserted. A bounded or
    /// approximate search without matches is not a proof that none exist;
    /// `truncated` and `approximate` report it.
    pub result: SearchResult,
}

impl SearchIndex {
    /// An encoder for this index's queries, usable without the index.
    pub fn encoder(&self) -> QueryEncoder {
        QueryEncoder {
            schema: self.schema().clone(),
            embedder: self.vectors.as_ref().and(self.embedder.clone()),
            cache: self.cache.clone(),
        }
    }

    /// Searches with embeddings from [`QueryEncoder::encode`].
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::search`], `state.schema_mismatch` when the
    /// embeddings were computed for another schema or another number of
    /// records than one, and `state.model_mismatch` when they come from
    /// another model than the stored ones.
    pub fn search_encoded(
        &self,
        query: &PreparedRecord,
        vectors: Option<&RecordVectors>,
        options: SearchOptions,
        exhaustive: bool,
    ) -> Result<SearchResult> {
        check_options(options)?;
        if !query.is_prepared_with(self.schema()) {
            return Err(Error::schema_mismatch());
        }
        self.check_encoded(vectors, 1)?;
        let query = Query {
            record: query,
            vectors: vectors.map_or_else(Vec::new, |vectors| vectors.record(0)),
            slot: None,
            min_slot: 0,
            dense: None,
        };
        if exhaustive {
            let slots = self
                .partition(query.record)
                .iter()
                .map(|&slot| slot as usize);
            let mut result = SearchResult::default();
            self.score(&query, slots, options, &mut result)?;
            Ok(result)
        } else {
            self.search_candidates(&query, options)
        }
    }

    /// Searches for `record` and inserts it only if no record reaches
    /// `options.min_score`, in one exclusive borrow: two calls on one index
    /// never both insert the same duplicate. `vectors` are its embeddings
    /// from [`QueryEncoder::encode`], reused for the insertion.
    ///
    /// A record equal on every selected field gives [`Decision::Matched`];
    /// other records reaching the minimum give [`Decision::Review`]; neither
    /// is inserted. Otherwise the record is inserted, even after a bounded or
    /// approximate search, which the result reports.
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::search_encoded`] and [`SearchIndex::add`]: on error
    /// nothing is inserted.
    pub fn check_or_add(
        &mut self,
        record: PreparedRecord,
        vectors: Option<RecordVectors>,
        options: SearchOptions,
        exhaustive: bool,
    ) -> Result<CheckOrAdd> {
        check_options(options)?;
        if !record.is_prepared_with(self.schema()) {
            return Err(Error::schema_mismatch());
        }
        self.check_encoded(vectors.as_ref(), 1)?;
        // Compaction renumbers slots: it runs before the search, so that the
        // decision and the insertion see the same layout.
        self.compact_if_needed(1)?;
        let result = self.search_encoded(&record, vectors.as_ref(), options, exhaustive)?;
        let equal = self
            .exact
            .matching_slots(&record)?
            .into_iter()
            .any(|slot| self.store().is_active(slot));
        if equal || result.total_matches > 0 {
            let decision = if equal {
                Decision::Matched
            } else {
                Decision::Review
            };
            return Ok(CheckOrAdd {
                decision,
                slot: None,
                result,
            });
        }
        let slots = self.add_preembedded(vec![record], vectors)?;
        Ok(CheckOrAdd {
            decision: Decision::Inserted,
            slot: Some(slots.start),
            result,
        })
    }

    /// [`SearchIndex::add`] with embeddings from
    /// [`QueryEncoder::encode_batch`], so that a caller holding a lock does
    /// not run the model under it.
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::add`], `state.schema_mismatch` when the embeddings
    /// were computed for another schema or do not cover exactly `records`,
    /// and `state.model_mismatch` when they come from another model: on
    /// error the index is unchanged.
    pub fn add_encoded(
        &mut self,
        records: Vec<PreparedRecord>,
        vectors: Option<RecordVectors>,
    ) -> Result<std::ops::Range<usize>> {
        self.check_encoded(vectors.as_ref(), records.len())?;
        self.compact_if_needed(records.len())?;
        self.add_preembedded(records, vectors)
    }

    /// Embeddings are present exactly when the index stores some, come
    /// from its model and schema, and cover exactly `records` records: a
    /// vector is never used for a record or a field it was not computed for.
    fn check_encoded(&self, vectors: Option<&RecordVectors>, records: usize) -> Result<()> {
        match (&self.vectors, vectors) {
            (Some(stored), Some(encoded)) => {
                if !encoded.covers(self.schema().preparation_fingerprint(), records) {
                    return Err(Error::schema_mismatch());
                }
                if stored.model() != encoded.model() {
                    return Err(Error::model_mismatch(
                        &stored.model().name(),
                        &encoded.model().name(),
                    ));
                }
                Ok(())
            }
            (None, None) => Ok(()),
            (Some(stored), None) => Err(Error::model_mismatch(&stored.model().name(), "none")),
            (None, Some(encoded)) => Err(Error::model_mismatch("none", &encoded.model().name())),
        }
    }
}
