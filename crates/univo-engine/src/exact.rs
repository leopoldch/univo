//! Exact indexing with verified equality and bounded search output.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use univo_core::{
    ExactMatch, PreparedRecord, RecordId, Result, Schema, exact_fingerprint, exact_match,
    has_exact_evidence, is_exact_match,
};

use crate::store::{RecordStore, id_text_bytes};

/// Verified equality over the active versions of its own [`RecordStore`].
/// Slots identify records without external IDs.
#[derive(Debug)]
pub struct ExactIndex {
    store: RecordStore,
    buckets: HashMap<u64, Bucket>,
}

#[derive(Debug, Default)]
struct Bucket {
    slots: Vec<usize>,
    ids: Ids,
}

/// The slots of a bucket's records by typed ID. Most buckets hold one
/// record: it is stored inline, and a tree is built for the second.
#[derive(Debug, Default)]
enum Ids {
    #[default]
    None,
    One(RecordId, usize),
    Many(BTreeMap<RecordId, usize>),
}

impl Ids {
    fn insert(&mut self, id: &RecordId, slot: usize) {
        match self {
            Self::None => *self = Self::One(id.clone(), slot),
            Self::One(only, only_slot) if only == id => *only_slot = slot,
            Self::One(..) => {
                let Self::One(only, only_slot) = std::mem::take(self) else {
                    unreachable!("matched above")
                };
                *self = Self::Many(BTreeMap::from([(only, only_slot), (id.clone(), slot)]));
            }
            Self::Many(ids) => {
                ids.insert(id.clone(), slot);
            }
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Slots in typed ID order, without allocating.
    fn values(&self) -> impl Iterator<Item = usize> + '_ {
        let (one, many) = match self {
            Self::None => (None, None),
            Self::One(_, slot) => (Some(*slot), None),
            Self::Many(ids) => (None, Some(ids.values().copied())),
        };
        one.into_iter().chain(many.into_iter().flatten())
    }

    fn keys(&self) -> impl Iterator<Item = &RecordId> + '_ {
        let (one, many) = match self {
            Self::None => (None, None),
            Self::One(id, _) => (Some(id), None),
            Self::Many(ids) => (None, Some(ids.keys())),
        };
        one.into_iter().chain(many.into_iter().flatten())
    }

    /// Heap bytes beyond the bucket itself: tree nodes for several IDs.
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Many(ids) => ids.len() * (size_of::<RecordId>() + size_of::<usize>()) * 3 / 2,
            Self::None | Self::One(..) => 0,
        }
    }
}

impl Bucket {
    fn insert(&mut self, slot: usize, id: Option<&RecordId>) {
        if let Some(id) = id {
            self.ids.insert(id, slot);
        } else {
            self.slots.push(slot);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactHit {
    pub slot: usize,
    pub evidence: ExactMatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExactSearch {
    pub matches: Vec<ExactHit>,
    pub total_matches: usize,
    /// Bucket entries visited, including a query's excluded ID.
    pub candidates_examined: usize,
}

impl ExactSearch {
    pub fn truncated(&self) -> bool {
        self.total_matches > self.matches.len()
    }
}

impl ExactIndex {
    pub fn new(schema: Schema, max_records: usize) -> Self {
        Self {
            store: RecordStore::new(schema, max_records),
            buckets: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    pub fn schema(&self) -> &Schema {
        self.store.schema()
    }

    pub fn record(&self, slot: usize) -> Option<&PreparedRecord> {
        self.store.get(slot)
    }

    /// Estimated heap bytes of the fingerprint postings and the ID set,
    /// excluding the records. Ordered postings count each B-tree entry
    /// with half its size again for node overhead.
    pub fn heap_bytes(&self) -> usize {
        let buckets: usize = self
            .buckets
            .values()
            .map(|bucket| {
                bucket.slots.capacity() * size_of::<usize>()
                    + bucket.ids.heap_bytes()
                    + id_text_bytes(bucket.ids.keys())
            })
            .sum();
        buckets
            + self.buckets.capacity() * (size_of::<u64>() + size_of::<Bucket>() + 1)
            + self.store.id_heap_bytes()
    }

    /// The records this index covers.
    pub fn store(&self) -> &RecordStore {
        &self.store
    }

    /// Validates the whole batch before inserting it; returns its slots.
    ///
    /// # Errors
    /// Rejects incompatible schemas, repeated IDs and batches exceeding capacity,
    /// leaving the index unchanged; see [`RecordStore::append`].
    pub fn add(&mut self, records: Vec<PreparedRecord>) -> Result<Range<usize>> {
        let slots = self.store.append(records)?;
        self.index_slots(slots.clone())?;
        Ok(slots)
    }

    pub(crate) fn upsert(&mut self, records: Vec<PreparedRecord>) -> Result<Range<usize>> {
        let slots = self.store.upsert(records)?;
        self.index_slots(slots.clone())?;
        Ok(slots)
    }

    pub(crate) fn remove(&mut self, ids: &[RecordId]) -> Result<Vec<RecordId>> {
        self.store.remove(ids)
    }

    fn index_slots(&mut self, slots: Range<usize>) -> Result<()> {
        for slot in slots.clone() {
            let record = &self.store.records()[slot];
            let fingerprint = exact_fingerprint(self.store.schema(), record)?;
            self.buckets
                .entry(fingerprint)
                .or_default()
                .insert(slot, record.id());
        }
        Ok(())
    }

    /// Slots of every record equal to `query` on every field, excluding the
    /// query's typed ID, without building explanations: the cost of a large
    /// group of duplicates is one slot each.
    ///
    /// # Errors
    /// Rejects a query prepared with an incompatible schema.
    pub fn matching_slots(&self, query: &PreparedRecord) -> Result<Vec<usize>> {
        // A query without exact evidence matches nothing: its bucket, which
        // holds every blank record, is not scanned.
        if !has_exact_evidence(self.store.schema(), query)? {
            return Ok(Vec::new());
        }
        let fingerprint = exact_fingerprint(self.store.schema(), query)?;
        let Some(bucket) = self.buckets.get(&fingerprint) else {
            return Ok(Vec::new());
        };
        let records = self.store.records();
        let mut slots = Vec::new();
        for slot in bucket.slots.iter().copied().chain(bucket.ids.values()) {
            if !self.store.is_active(slot) {
                continue;
            }
            let record = &records[slot];
            if query.id().is_some() && query.id() == record.id() {
                continue;
            }
            if is_exact_match(self.store.schema(), query, record)? {
                slots.push(slot);
            }
        }
        Ok(slots)
    }

    /// Verifies every record in the fingerprint bucket, including collisions.
    /// Output is ordered by typed ID, or slot when IDs are absent.
    ///
    /// # Errors
    /// Rejects a query prepared with an incompatible schema.
    pub fn search(&self, query: &PreparedRecord, limit: usize) -> Result<ExactSearch> {
        let fingerprint = exact_fingerprint(self.store.schema(), query)?;
        match self.buckets.get(&fingerprint) {
            Some(bucket) if bucket.ids.is_empty() => {
                self.search_slots(query, bucket.slots.iter().copied(), limit)
            }
            Some(bucket) => self.search_slots(query, bucket.ids.values(), limit),
            None => Ok(ExactSearch::default()),
        }
    }

    fn search_slots(
        &self,
        query: &PreparedRecord,
        slots: impl Iterator<Item = usize>,
        limit: usize,
    ) -> Result<ExactSearch> {
        let mut result = ExactSearch::default();
        for slot in slots {
            if !self.store.is_active(slot) {
                continue;
            }
            result.candidates_examined += 1;
            let record = &self.store.records()[slot];
            if query.id().is_some() && query.id() == record.id() {
                continue;
            }
            let Some(evidence) = exact_match(self.store.schema(), query, record)? else {
                continue;
            };
            result.total_matches += 1;
            if result.matches.len() < limit {
                result.matches.push(ExactHit { slot, evidence });
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use univo_core::{FieldPath, Map, Text, Value, prepare};

    fn object(tenant: &str, name: &str) -> Value {
        Value::Map(
            Map::new(vec![
                ("tenant".into(), Value::Str(tenant.into())),
                ("name".into(), Value::Str(name.into())),
            ])
            .unwrap(),
        )
    }

    #[test]
    fn ids_stay_inline_until_a_second_one_arrives() {
        let (a, b, c) = (
            RecordId::Str("a".into()),
            RecordId::Int(2),
            RecordId::Str("c".into()),
        );
        let mut ids = Ids::default();
        assert!(ids.is_empty());
        ids.insert(&c, 7);
        ids.insert(&c, 8);
        assert!(matches!(ids, Ids::One(_, 8)));
        assert_eq!(ids.heap_bytes(), 0);
        ids.insert(&a, 1);
        ids.insert(&b, 2);
        ids.insert(&a, 3);
        assert!(matches!(ids, Ids::Many(_)));
        // Typed ID order: strings before integers.
        assert_eq!(ids.values().collect::<Vec<_>>(), [3, 8, 2]);
        assert_eq!(ids.keys().cloned().collect::<Vec<_>>(), [a, c, b]);
        assert!(ids.heap_bytes() > 0);
    }

    #[test]
    fn collisions_never_bypass_content_or_partitions() {
        let schema = Schema::builder()
            .partition_by(FieldPath::parse("tenant").unwrap())
            .text(FieldPath::parse("name").unwrap(), Text::default())
            .build()
            .unwrap();
        let query = prepare(&schema, &object("a", "same")).unwrap();
        let records = [("a", "same"), ("a", "different"), ("b", "same")]
            .iter()
            .map(|(tenant, name)| prepare(&schema, &object(tenant, name)).unwrap())
            .collect();
        let fingerprint = exact_fingerprint(&schema, &query).unwrap();
        let mut index = ExactIndex::new(schema, 3);
        index.add(records).unwrap();
        index.buckets.clear();
        index.buckets.insert(
            fingerprint,
            Bucket {
                slots: vec![0, 1, 2],
                ..Bucket::default()
            },
        );
        let result = index.search(&query, 3).unwrap();
        assert_eq!(result.total_matches, 1);
        assert_eq!(result.candidates_examined, 3);
        assert_eq!(result.matches[0].slot, 0);
    }
}
