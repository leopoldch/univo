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
    Comparison, Error, FieldKind, PreparedRecord, RecordId, Result, Schema, TextMode,
    compare_with_vectors, partition_fingerprint,
};
use univo_embed::Embedder;

use crate::dense::within;
use crate::exact::ExactIndex;
use crate::lexical::LexicalIndex;
use crate::store::RecordStore;
use crate::values::ValueIndex;
use crate::vectors::{RecordVectors, embed_records, is_embedded};

/// Relative margin by which lanes lower the minimum score, so that rounding
/// in the weighted mean never drops a record that reaches it. Relative, so
/// that the lowered minimum stays positive for every positive minimum.
const ROUNDING_MARGIN: f32 = 1e-6;

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
    vectors: Option<RecordVectors>,
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
        let vectors = match &embedder {
            Some(model) if !semantic.is_empty() => {
                Some(embed_records(exact.schema(), &[], model.as_ref())?)
            }
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
        })
    }

    pub fn len(&self) -> usize {
        self.exact.len()
    }

    pub fn is_empty(&self) -> bool {
        self.exact.is_empty()
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
        let batch = match (&self.embedder, &self.vectors) {
            (Some(model), Some(vectors)) => {
                let batch = embed_records(self.schema(), &records, model.as_ref())?;
                vectors.check_extend(&batch)?;
                Some(batch)
            }
            _ => None,
        };
        // Every check is done: from here on nothing can fail but the store's
        // own validation, which changes nothing when it fails.
        let slots = self.exact.add(records)?;
        if let (Some(vectors), Some(batch)) = (&mut self.vectors, batch) {
            vectors.extend(batch)?;
        }
        let store = self.exact.store();
        for index in &mut self.lexical {
            index.update(store)?;
        }
        for index in &mut self.values {
            index.update(store)?;
        }
        for slot in slots.clone() {
            let partition = partition_fingerprint(&store.records()[slot]);
            // Within the limit checked by `new`.
            let slot = u32::try_from(slot).unwrap_or(u32::MAX);
            self.partitions.entry(partition).or_default().push(slot);
        }
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

    fn stored_query(&self, slot: usize) -> Result<Query<'_>> {
        let record = self
            .store()
            .get(slot)
            .ok_or_else(|| Error::index_capacity(slot.saturating_add(1), self.len()))?;
        let vectors = self
            .vectors
            .as_ref()
            .map_or_else(Vec::new, |vectors| vectors.record(slot));
        Ok(Query {
            record,
            vectors,
            slot: Some(slot),
        })
    }

    /// Embeds the query, checking that its embeddings come from the model
    /// of the stored ones.
    fn embed_query(&self, query: &PreparedRecord) -> Result<Option<RecordVectors>> {
        let (Some(model), Some(vectors)) = (&self.embedder, &self.vectors) else {
            return Ok(None);
        };
        let embedded = embed_records(self.schema(), std::slice::from_ref(query), model.as_ref())?;
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
                    partition.iter().map(|&slot| slot as usize),
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
            if Some(slot) == query.slot {
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
            let Some(comparison) = compare_with_vectors(
                self.schema(),
                query.record,
                &query.vectors,
                record,
                &vectors,
            )?
            else {
                continue;
            };
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
