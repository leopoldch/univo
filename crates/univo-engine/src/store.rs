//! Append-only storage of the prepared records indexes cover.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use univo_core::{Error, PreparedRecord, RecordId, Result, Schema};

/// Prepared records of one schema, by slot, with unique typed IDs and an
/// explicit record limit. Records are only appended: never reordered,
/// replaced or removed, so an index built from a store stays valid for the
/// records it covers. Each store has an identity, which indexes check in
/// constant time; it is deliberately not `Clone`.
#[derive(Debug)]
pub struct RecordStore {
    id: u64,
    schema: Schema,
    max_records: usize,
    records: Vec<PreparedRecord>,
    ids: HashSet<RecordId>,
}

impl RecordStore {
    pub fn new(schema: Schema, max_records: usize) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            schema,
            max_records,
            records: Vec::new(),
            ids: HashSet::new(),
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub fn max_records(&self) -> usize {
        self.max_records
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn get(&self, slot: usize) -> Option<&PreparedRecord> {
        self.records.get(slot)
    }

    /// Estimated heap bytes of the ID set, string IDs included; the records
    /// themselves are not counted.
    pub fn id_heap_bytes(&self) -> usize {
        self.ids.capacity() * (size_of::<RecordId>() + 1) + id_text_bytes(self.ids.iter())
    }

    /// The records, by slot.
    pub fn records(&self) -> &[PreparedRecord] {
        &self.records
    }

    /// Appends `records` as the next slots and returns them. The whole
    /// batch is checked first: on error the store is unchanged.
    ///
    /// # Errors
    ///
    /// Returns `limit.index_capacity` beyond the record limit, and, located
    /// at the offending record, `state.schema_mismatch` for a record
    /// prepared with another schema and `record.duplicate_id` for an ID
    /// already stored or repeated in the batch.
    pub fn append(&mut self, records: Vec<PreparedRecord>) -> Result<Range<usize>> {
        let start = self.records.len();
        let end = start.saturating_add(records.len());
        if records.len() > self.max_records.saturating_sub(start) {
            return Err(Error::index_capacity(end, self.max_records));
        }
        let mut batch = HashSet::new();
        for (index, record) in records.iter().enumerate() {
            let locate = |error: Error| error.with_record(index, record.id().cloned());
            if !record.is_prepared_with(&self.schema) {
                return Err(locate(Error::schema_mismatch()));
            }
            if let Some(id) = record.id()
                && (self.ids.contains(id) || !batch.insert(id))
            {
                return Err(locate(Error::duplicate_id()));
            }
        }
        self.ids
            .extend(records.iter().filter_map(|record| record.id().cloned()));
        self.records.extend(records);
        Ok(start..end)
    }
}

/// Heap bytes of the text of string IDs.
pub(crate) fn id_text_bytes<'a>(ids: impl Iterator<Item = &'a RecordId>) -> usize {
    ids.map(|id| match id {
        RecordId::Str(text) => text.len(),
        RecordId::Int(_) => 0,
    })
    .sum()
}
