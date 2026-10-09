//! Append-only storage of the prepared records an index covers.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use univo_core::{Error, PreparedRecord, Result, Schema};

/// Prepared records of one schema, by slot. Records are only appended:
/// never reordered, replaced or removed, so an index built from a store
/// stays valid for the records it covers. Each store has an identity, which
/// indexes check in constant time; it is deliberately not `Clone`.
#[derive(Debug)]
pub struct RecordStore {
    id: u64,
    schema: Schema,
    records: Vec<PreparedRecord>,
}

impl RecordStore {
    pub fn new(schema: Schema) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            schema,
            records: Vec::new(),
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
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

    /// The records, by slot.
    pub fn records(&self) -> &[PreparedRecord] {
        &self.records
    }

    /// Appends `records` as the next slots and returns them.
    ///
    /// # Errors
    ///
    /// Returns `state.schema_mismatch`, leaving the store unchanged, when a
    /// record was prepared with another schema.
    pub fn append(&mut self, records: Vec<PreparedRecord>) -> Result<Range<usize>> {
        if !records
            .iter()
            .all(|record| record.is_prepared_with(&self.schema))
        {
            return Err(Error::schema_mismatch());
        }
        let start = self.records.len();
        self.records.extend(records);
        Ok(start..self.records.len())
    }
}
