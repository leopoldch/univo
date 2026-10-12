//! Search for the records whose score against a query reaches a minimum,
//! by the union of one candidate lane per field, then scoring.
//!
//! A record's score is a weighted mean of its field similarities, so a
//! score of at least `s` needs one field with a similarity of at least `s`.
//! Each lane finds every record that can reach `s` on its field:
//!
//! - lexical and hybrid text: the trigram index of [`crate::lexical`];
//! - semantic text: exhaustive vector search in the query's partition;
//! - exact values: equal values, or for objects one equal leaf;
//!
//! Lanes lower the minimum by a relative 10⁻⁶ against rounding, and
//! records equal to the query on every field are always included
//! through the exact index, whatever the work limit. Without truncation the
//! result is exactly the one of [`SearchIndex::search_exhaustive`], which
//! scores every record of the partition: the oracle, at `O(n)` per query.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::ops::Range;
use std::sync::Arc;

use univo_core::{
    Comparison, Error, FieldKind, PreparedRecord, RecordId, Result, Schema, Signal, TextMode,
    compare_with_vectors, has_value, is_exact_match, partition_fingerprint,
};
use univo_embed::Embedder;

use crate::dense::within;
use crate::exact::ExactIndex;
use crate::lexical::LexicalIndex;
use crate::store::RecordStore;
use crate::values::ValueIndex;
use crate::vectors::{RecordVectors, is_embedded};
use crate::{EmbeddingCache, embed_records_cached};

/// Relative margin by which lanes lower the minimum score, so that rounding
/// in the weighted mean never drops a record that reaches it. Relative, so
/// that the lowered minimum stays positive for every positive minimum.
const ROUNDING_MARGIN: f32 = 1e-6;

/// Search decision version, independent of pairwise scoring. Version 1
/// preserves verified whole-record equality when embeddings give no evidence.
pub const SEARCH_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SearchOptions {
    /// Records scoring at least this are returned; `0 < min_score ≤ 1`.
    pub min_score: f32,
    /// Matches returned; `total_matches` still counts all of them.
    pub limit: usize,
    /// Index entries one lane may read before stopping, which may lose
    /// matches and is reported as truncation. The exact lane and the
    /// semantic lane, which scans the partition, are not limited.
    pub max_entries: usize,
}

/// A record whose score reaches the minimum, with its explanation.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchMatch {
    pub slot: usize,
    pub comparison: Comparison,
}

/// Candidates each lane proposed, before deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LaneCounts {
    pub exact: usize,
    pub lexical: usize,
    pub semantic: usize,
    pub values: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SearchResult {
    /// Highest score first, then by ID, or by slot without IDs.
    pub matches: Vec<SearchMatch>,
    /// Every match found, including those beyond the limit.
    pub total_matches: usize,
    /// Distinct records scored.
    pub candidates: usize,
    pub lanes: LaneCounts,
    /// A lane reached its work limit: matches may be missing.
    pub truncated: bool,
}

/// Records with their candidate lanes, appended in batches.
pub struct SearchIndex {
    exact: ExactIndex,
    lexical: Vec<LexicalIndex>,
    values: Vec<ValueIndex>,
    /// Semantic fields, by schema index.
    semantic: Vec<usize>,
    embedder: Option<Arc<dyn Embedder>>,
    pub(crate) vectors: Option<RecordVectors>,
    cache: Option<Arc<EmbeddingCache>>,
    /// Slots of each partition, in order.
    partitions: HashMap<u64, Vec<u32>>,
}

impl std::fmt::Debug for SearchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchIndex")
            .field("records", &self.len())
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

/// The query, its embeddings and what it must not match: its own slot and
/// typed ID.
struct Query<'a> {
    record: &'a PreparedRecord,
    vectors: Vec<Option<&'a [f32]>>,
    slot: Option<usize>,
    min_slot: usize,
}

impl SearchIndex {
    /// An empty index of at most `max_records` records, at most 2^32.
    /// `embedder` encodes the semantic fields; it is required when the
    /// schema has any.
    ///
    /// # Errors
    ///
    /// Returns `limit.index_capacity` for a limit above 2^32 records, and
    /// `model.unavailable` for a schema with semantic fields and no embedder.
    pub fn new(
        schema: Schema,
        max_records: usize,
        embedder: Option<Arc<dyn Embedder>>,
    ) -> Result<Self> {
        // Slots are stored as u32: checking the limit here keeps every
        // later step of `add` infallible on that account.
        if u32::try_from(max_records).is_err() {
            return Err(Error::index_capacity(max_records, u32::MAX as usize));
        }
        let semantic: Vec<usize> = (0..schema.fields().len())
            .filter(|&index| is_embedded(&schema, index))
            .collect();
        if !semantic.is_empty() && embedder.is_none() {
            return Err(Error::model_not_available(
                "none",
                "the schema has semantic fields, which need a model",
            ));
        }
        let exact = ExactIndex::new(schema, max_records);
        let mut lexical = Vec::new();
        let mut values = Vec::new();
        for (index, field) in exact.schema().fields().iter().enumerate() {
            match field.kind {
                FieldKind::Text(text) if text.mode != TextMode::Semantic => {
                    lexical.push(LexicalIndex::new(exact.store(), index)?);
                }
                FieldKind::Exact(_) => values.push(ValueIndex::new(index)),
                FieldKind::Text(_) => {}
            }
        }
        if let Some(model) = &embedder
            && !semantic.is_empty()
            && model.descriptor().dimension == 0
        {
            return Err(Error::corrupt_model(
                &model.descriptor().name(),
                "zero embedding dimension",
            ));
        }
        let vectors = match &embedder {
            Some(model) if !semantic.is_empty() => Some(RecordVectors::empty(
                exact.schema(),
                model.descriptor().clone(),
            )),
            _ => None,
        };
        Ok(Self {
            exact,
            lexical,
            values,
            semantic,
            embedder,
            vectors,
            partitions: HashMap::new(),
            cache: None,
        })
    }

    /// Share a bounded semantic cache across compatible indexes.
    ///
    /// # Errors
    /// Refuses a cache bound to another model.
    pub fn with_embedding_cache(mut self, cache: Arc<EmbeddingCache>) -> Result<Self> {
        if self
            .embedder
            .as_ref()
            .is_none_or(|m| m.descriptor() != cache.model())
        {
            return Err(Error::model_mismatch("index", &cache.model().name()));
        }
        self.cache = Some(cache);
        Ok(self)
    }

    pub fn len(&self) -> usize {
        self.store().active_len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn schema(&self) -> &Schema {
        self.exact.schema()
    }

    pub fn store(&self) -> &RecordStore {
        self.exact.store()
    }

    /// Embeds and appends `records`, returning their slots. The batch is
    /// embedded and every check is made before anything is stored: on error
    /// the index is unchanged.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`RecordStore::append`] and of the embedder,
    /// and `state.model_mismatch` when the embedder no longer describes the
    /// model of the stored embeddings.
    pub fn add(&mut self, records: Vec<PreparedRecord>) -> Result<Range<usize>> {
        self.compact_if_needed(records.len())?;
        let batch = match (&self.embedder, &self.vectors) {
            (Some(model), Some(_)) => {
                let batch = embed_records_cached(
                    self.schema(),
                    &records,
                    model.as_ref(),
                    self.cache.as_deref(),
                )?;
                Some(batch)
            }
            _ => None,
        };
        self.add_preembedded(records, batch)
    }

    /// Replace typed IDs atomically after validation and encoding. New IDs are inserted.
    ///
    /// # Errors
    /// Rejects absent/repeated IDs, incompatible schemas/models, capacity or encoding failures.
    pub fn upsert(&mut self, records: Vec<PreparedRecord>) -> Result<Vec<usize>> {
        self.store().replacements(&records)?;
        if records.is_empty() {
            return Ok(Vec::new());
        }
        self.compact_if_needed(records.len())?;
        let old = self.store().replacements(&records)?;
        let batch = match (&self.embedder, &self.vectors) {
            (Some(model), Some(previous)) => {
                let mut batch = crate::vectors::embed_records_selected(
                    self.schema(),
                    &records,
                    model.as_ref(),
                    self.cache.as_deref(),
                    |position, field| {
                        old[position].is_none_or(|slot| {
                            records[position].fields()[field].state
                                != self.store().records()[slot].fields()[field].state
                        })
                    },
                )?;
                batch.reuse(previous, &old, &records, self.store())?;
                previous.check_extend(&batch)?;
                Some(batch)
            }
            _ => None,
        };
        let slots = self.exact.upsert(records)?;
        self.publish_vectors(slots.clone(), batch)?;
        self.update_lanes(slots.clone())?;
        Ok(slots.collect())
    }

    /// Missing IDs are returned in input order. Removed slots are masked in every lane.
    ///
    /// # Errors
    /// Requires a schema with external IDs.
    pub fn remove(&mut self, ids: &[RecordId]) -> Result<Vec<RecordId>> {
        self.exact.remove(ids)
    }

    /// Rebuild when obsolete slots would grow beyond an amortized bound.
    /// Layout changes do not change the logical generation or encode records.
    fn compact_if_needed(&mut self, incoming: usize) -> Result<()> {
        let dead = self.store().len() - self.len();
        if dead == 0
            || (dead < self.len().div_ceil(2).max(64)
                && self.store().len().saturating_add(incoming)
                    <= self
                        .store()
                        .max_records()
                        .saturating_mul(2)
                        .min(u32::MAX as usize))
        {
            return Ok(());
        }
        let slots: Vec<_> = self.store().active_slots().collect();
        let records = slots
            .iter()
            .map(|&slot| self.store().records()[slot].clone())
            .collect();
        let vectors = self
            .vectors
            .as_ref()
            .map(|vectors| vectors.select(&slots))
            .transpose()?;
        let mut fresh = Self::new(
            self.schema().clone(),
            self.store().max_records(),
            self.embedder.clone(),
        )?;
        fresh.cache.clone_from(&self.cache);
        fresh.add_preembedded(records, vectors)?;
        *self = fresh;
        Ok(())
    }

    fn publish_vectors(&mut self, slots: Range<usize>, batch: Option<RecordVectors>) -> Result<()> {
        if let (Some(vectors), Some(batch)) = (&mut self.vectors, batch) {
            if slots.start == 0 {
                *vectors = batch;
            } else {
                vectors.extend(batch)?;
            }
        }
        Ok(())
    }

    fn update_lanes(&mut self, slots: Range<usize>) -> Result<()> {
        let store = self.exact.store();
        for index in &mut self.lexical {
            index.update(store)?;
        }
        for index in &mut self.values {
            index.update(store)?;
        }
        for slot in slots {
            let partition = partition_fingerprint(&store.records()[slot]);
            self.partitions
                .entry(partition)
                .or_default()
                .push(u32::try_from(slot).unwrap_or(u32::MAX));
        }
        Ok(())
    }

    pub(crate) fn add_preembedded(
        &mut self,
        records: Vec<PreparedRecord>,
        batch: Option<RecordVectors>,
    ) -> Result<Range<usize>> {
        match (&self.vectors, &batch) {
            (Some(vectors), Some(batch)) => vectors.check_extend(batch)?,
            (None, None) => {}
            _ => return Err(Error::schema_mismatch()),
        }
        // Every check is done: from here on nothing can fail but the store's
        // own validation, which changes nothing when it fails.
        let slots = self.exact.add(records)?;
        self.publish_vectors(slots.clone(), batch)?;
        self.update_lanes(slots.clone())?;
        Ok(slots)
    }

    /// Estimated heap bytes of every index, the ID set and the embeddings,
    /// excluding the records and the model.
    pub fn heap_bytes(&self) -> usize {
        self.exact.heap_bytes()
            + self
                .lexical
                .iter()
                .map(LexicalIndex::heap_bytes)
                .sum::<usize>()
            + self
                .values
                .iter()
                .map(ValueIndex::heap_bytes)
                .sum::<usize>()
            + self.vectors.as_ref().map_or(0, RecordVectors::heap_bytes)
            + self
                .partitions
                .values()
                .map(|slots| slots.capacity() * 4)
                .sum::<usize>()
    }

    /// Records whose score against `query` reaches `options.min_score`,
    /// excluding records with the query's typed ID.
    ///
    /// # Errors
    ///
    /// Returns `parameter.threshold` for a minimum score outside `(0, 1]`,
    /// `state.schema_mismatch` for a query of another schema, and the
    /// embedder's errors.
    pub fn search(&self, query: &PreparedRecord, options: SearchOptions) -> Result<SearchResult> {
        let embedded = self.embed_query(query)?;
        let query = Query {
            record: query,
            vectors: embedded
                .as_ref()
                .map_or_else(Vec::new, |vectors| vectors.record(0)),
            slot: None,
            min_slot: 0,
        };
        self.search_candidates(&query, options)
    }

    /// Records whose score against the record at `slot` reaches the
    /// minimum, excluding that record; its stored embeddings are reused.
    ///
    /// # Errors
    ///
    /// Returns `parameter.threshold` for a minimum score outside `(0, 1]`
    /// and `limit.index_capacity` for a slot beyond the records.
    pub fn search_slot(&self, slot: usize, options: SearchOptions) -> Result<SearchResult> {
        let query = self.stored_query(slot)?;
        self.search_candidates(&query, options)
    }

    /// The oracle of [`SearchIndex::search`]: every record of the query's
    /// partition is scored, with no index and no work limit.
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::search`].
    pub fn search_exhaustive(
        &self,
        query: &PreparedRecord,
        options: SearchOptions,
    ) -> Result<SearchResult> {
        check_options(options)?;
        if !query.is_prepared_with(self.schema()) {
            return Err(Error::schema_mismatch());
        }
        let embedded = self.embed_query(query)?;
        let query = Query {
            record: query,
            vectors: embedded
                .as_ref()
                .map_or_else(Vec::new, |vectors| vectors.record(0)),
            slot: None,
            min_slot: 0,
        };
        let slots = self
            .partition(query.record)
            .iter()
            .map(|&slot| slot as usize);
        let mut result = SearchResult::default();
        self.score(&query, slots, options, &mut result)?;
        Ok(result)
    }

    /// [`SearchIndex::search_exhaustive`] for the record at `slot`.
    ///
    /// # Errors
    ///
    /// As [`SearchIndex::search_slot`].
    pub fn search_slot_exhaustive(
        &self,
        slot: usize,
        options: SearchOptions,
    ) -> Result<SearchResult> {
        check_options(options)?;
        let query = self.stored_query(slot)?;
        let slots = self
            .partition(query.record)
            .iter()
            .map(|&slot| slot as usize);
        let mut result = SearchResult::default();
        self.score(&query, slots, options, &mut result)?;
        Ok(result)
    }

    pub(crate) fn pair_count(&self) -> usize {
        self.partitions
            .values()
            .map(|slots| {
                let count = slots
                    .iter()
                    .filter(|&&slot| self.store().is_active(slot as usize))
                    .count();
                count.saturating_mul(count.saturating_sub(1)) / 2
            })
            .fold(0, usize::saturating_add)
    }

    /// Conservative count of dense semantic lane entries for all queries.
    /// Includes both directions, self entries and retained tombstones walked
    /// by active queries, for every semantic field.
    pub(crate) fn semantic_scan_count(&self) -> usize {
        self.partitions
            .values()
            .map(|slots| {
                let count = slots
                    .iter()
                    .filter(|&&slot| self.store().is_active(slot as usize))
                    .count();
                count.saturating_mul(slots.len())
            })
            .fold(0, usize::saturating_add)
            .saturating_mul(self.semantic.len())
    }

    pub(crate) fn search_pairs(
        &self,
        slot: usize,
        options: SearchOptions,
        exhaustive: bool,
    ) -> Result<SearchResult> {
        check_options(options)?;
        let mut query = self.stored_query(slot)?;
        query.min_slot = slot + 1;
        if !exhaustive {
            return self.search_candidates(&query, options);
        }
        let slots = self
            .partition(query.record)
            .iter()
            .map(|&slot| slot as usize);
        let mut result = SearchResult::default();
        self.score(&query, slots, options, &mut result)?;
        Ok(result)
    }

    fn stored_query(&self, slot: usize) -> Result<Query<'_>> {
        let record = self
            .store()
            .get(slot)
            .filter(|_| self.store().is_active(slot))
            .ok_or_else(|| Error::index_capacity(slot.saturating_add(1), self.len()))?;
        let vectors = self
            .vectors
            .as_ref()
            .map_or_else(Vec::new, |vectors| vectors.record(slot));
        Ok(Query {
            record,
            vectors,
            slot: Some(slot),
            min_slot: 0,
        })
    }

    /// Embeds the query, checking that its embeddings come from the model
    /// of the stored ones.
    fn embed_query(&self, query: &PreparedRecord) -> Result<Option<RecordVectors>> {
        let (Some(model), Some(vectors)) = (&self.embedder, &self.vectors) else {
            return Ok(None);
        };
        let embedded = embed_records_cached(
            self.schema(),
            std::slice::from_ref(query),
            model.as_ref(),
            self.cache.as_deref(),
        )?;
        if embedded.model() != vectors.model() {
            return Err(Error::model_mismatch(
                &vectors.model().name(),
                &embedded.model().name(),
            ));
        }
        Ok(Some(embedded))
    }

    fn partition(&self, record: &PreparedRecord) -> &[u32] {
        self.partitions
            .get(&partition_fingerprint(record))
            .map_or(&[], Vec::as_slice)
    }

    fn search_candidates(&self, query: &Query<'_>, options: SearchOptions) -> Result<SearchResult> {
        check_options(options)?;
        if !query.record.is_prepared_with(self.schema()) {
            return Err(Error::schema_mismatch());
        }
        let threshold = options.min_score * (1.0 - ROUNDING_MARGIN);
        let mut result = SearchResult::default();
        let mut candidates: Vec<usize> = Vec::new();

        let exact = self.exact.matching_slots(query.record)?;
        result.lanes.exact = exact.len();
        candidates.extend(exact);

        for index in &self.lexical {
            let found = index.search(self.store(), query.record, threshold, options.max_entries)?;
            result.truncated |= found.truncated;
            result.lanes.lexical += found.matches.len();
            candidates.extend(found.matches.iter().map(|found| found.slot));
        }

        if let Some(vectors) = &self.vectors {
            let partition = self.partition(query.record);
            for &field in &self.semantic {
                let (Some(field_vectors), Some(query_vector)) = (
                    vectors.field(field),
                    query.vectors.get(field).copied().flatten(),
                ) else {
                    continue;
                };
                let found = within(
                    field_vectors,
                    query_vector,
                    partition
                        .iter()
                        .map(|&slot| slot as usize)
                        .filter(|&slot| self.store().is_active(slot)),
                    threshold,
                    |slot| slot,
                );
                result.lanes.semantic += found.len();
                candidates.extend(found.iter().map(|neighbor| neighbor.slot));
            }
        }

        for index in &self.values {
            let mut read = 0usize;
            'lists: for slots in index.lists(self.store(), query.record) {
                for &slot in slots {
                    if read == options.max_entries {
                        result.truncated = true;
                        break 'lists;
                    }
                    read += 1;
                    result.lanes.values += 1;
                    candidates.push(slot as usize);
                }
            }
        }

        candidates.sort_unstable();
        candidates.dedup();
        self.score(query, candidates.into_iter(), options, &mut result)?;
        Ok(result)
    }

    /// Scores `slots`, each given once, against the query. Every match is
    /// counted, and only the best `limit` are kept, in a bounded heap: a
    /// large group of duplicates costs `O(limit)` memory, not one
    /// explanation per match.
    fn score(
        &self,
        query: &Query<'_>,
        slots: impl Iterator<Item = usize>,
        options: SearchOptions,
        result: &mut SearchResult,
    ) -> Result<()> {
        let records = self.store().records();
        let mut kept: BinaryHeap<Ranked<'_>> =
            BinaryHeap::with_capacity(options.limit.min(1024).saturating_add(1));
        for slot in slots {
            if !self.store().is_active(slot) || slot < query.min_slot || Some(slot) == query.slot {
                continue;
            }
            let record = &records[slot];
            if query.record.id().is_some() && query.record.id() == record.id() {
                continue;
            }
            result.candidates += 1;
            let vectors = self
                .vectors
                .as_ref()
                .map_or_else(Vec::new, |vectors| vectors.record(slot));
            let Some(mut comparison) = compare_with_vectors(
                self.schema(),
                query.record,
                &query.vectors,
                record,
                &vectors,
            )?
            else {
                continue;
            };
            if comparison.score.is_none() && is_exact_match(self.schema(), query.record, record)? {
                exact_evidence(self.schema(), query.record, &mut comparison);
            }
            let Some(score) = comparison.score.filter(|score| *score >= options.min_score) else {
                continue;
            };
            if comparison.vetoed {
                continue;
            }
            result.total_matches += 1;
            let candidate = Ranked {
                score,
                id: record.id(),
                found: SearchMatch { slot, comparison },
            };
            if kept.len() < options.limit {
                kept.push(candidate);
            } else if kept.peek().is_some_and(|worst| candidate < *worst) {
                kept.pop();
                kept.push(candidate);
            }
        }
        result.matches = kept
            .into_sorted_vec()
            .into_iter()
            .map(|ranked| ranked.found)
            .collect();
        Ok(())
    }
}

/// A kept match. The greatest is the worst: lowest score, then highest
/// typed ID (strings before integers), then highest slot.
struct Ranked<'a> {
    score: f32,
    id: Option<&'a RecordId>,
    found: SearchMatch,
}

impl Ord for Ranked<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then_with(|| self.id.cmp(&other.id))
            .then_with(|| self.found.slot.cmp(&other.found.slot))
    }
}

impl PartialOrd for Ranked<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Ranked<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Ranked<'_> {}

fn check_options(options: SearchOptions) -> Result<()> {
    if options.min_score > 0.0 && options.min_score <= 1.0 {
        Ok(())
    } else {
        Err(Error::invalid_threshold(options.min_score))
    }
}

/// Whole-record equality is a proof of normalized equality even if the
/// encoder knows none of the text. Missing fields remain missing evidence.
fn exact_evidence(schema: &Schema, record: &PreparedRecord, comparison: &mut Comparison) {
    let mut total = 0.0f64;
    let mut backed = 0.0f64;
    for ((field, value), evidence) in schema
        .fields()
        .iter()
        .zip(record.fields())
        .zip(comparison.fields.iter_mut())
    {
        let weight = f64::from(field.kind.weight());
        total += weight;
        if has_value(field, value) {
            evidence.similarity = Some(1.0);
            evidence.signal = Signal::Exact;
            evidence.evidence = 1.0;
            backed += weight;
        }
    }
    comparison.score = Some(1.0);
    #[allow(clippy::cast_possible_truncation)]
    {
        comparison.coverage = (backed / total) as f32;
    }
}
