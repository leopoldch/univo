//! Stable slots with typed active IDs and tombstones for obsolete versions.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use univo_core::{Error, PreparedRecord, RecordId, Result, Schema};

/// Prepared records in stable slots. Tombstones retain their data until the
/// owning search index compacts; secondary indexes filter the active mask.
/// Store identity prevents indexing another store by accident.
#[derive(Debug)]
pub struct RecordStore {
    id: u64,
    schema: Schema,
    max_records: usize,
    records: Vec<PreparedRecord>,
    ids: HashMap<RecordId, usize>,
    active: Vec<bool>,
    live: usize,
}

impl RecordStore {
    pub fn new(schema: Schema, max_records: usize) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            schema,
            max_records,
            records: Vec::new(),
            ids: HashMap::new(),
            active: Vec::new(),
            live: 0,
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

    pub fn active_len(&self) -> usize {
        self.live
    }

    pub fn is_active(&self, slot: usize) -> bool {
        self.active.get(slot).copied().unwrap_or(false)
    }

    pub fn active_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.active
            .iter()
            .enumerate()
            .filter_map(|(slot, &active)| active.then_some(slot))
    }

    pub fn contains_id(&self, id: &RecordId) -> bool {
        self.ids.contains_key(id)
    }

    pub(crate) fn slot(&self, id: &RecordId) -> Option<usize> {
        self.ids.get(id).copied()
    }

    /// Validate replacement IDs and capacity before encoding or publication.
    pub(crate) fn replacements(&self, records: &[PreparedRecord]) -> Result<Vec<Option<usize>>> {
        if self.schema.id_field().is_none() {
            return Err(Error::invalid_option("upsert", "a schema with id_field"));
        }
        let mut seen = HashSet::new();
        let mut slots = Vec::with_capacity(records.len());
        for (position, record) in records.iter().enumerate() {
            let locate = |error: Error| error.with_record(position, record.id().cloned());
            if !record.is_prepared_with(&self.schema) {
                return Err(locate(Error::schema_mismatch()));
            }
            let id = record
                .id()
                .ok_or_else(|| locate(Error::invalid_option("upsert", "records with IDs")))?;
            if !seen.insert(id) {
                return Err(locate(Error::duplicate_id()));
            }
            slots.push(self.slot(id));
        }
        let total = self
            .live
            .saturating_add(slots.iter().filter(|slot| slot.is_none()).count());
        if total > self.max_records {
            return Err(Error::index_capacity(total, self.max_records));
        }
        if self.len().saturating_add(records.len()) > u32::MAX as usize {
            return Err(Error::index_capacity(
                self.len().saturating_add(records.len()),
                u32::MAX as usize,
            ));
        }
        Ok(slots)
    }

    pub(crate) fn upsert(&mut self, records: Vec<PreparedRecord>) -> Result<Range<usize>> {
        let old = self.replacements(&records)?;
        for slot in old.into_iter().flatten() {
            self.remove_slot(slot);
        }
        self.append(records)
    }

    pub(crate) fn remove(&mut self, ids: &[RecordId]) -> Result<Vec<RecordId>> {
        if self.schema.id_field().is_none() {
            return Err(Error::invalid_option("remove", "a schema with id_field"));
        }
        let missing = ids
            .iter()
            .filter(|id| !self.ids.contains_key(*id))
            .cloned()
            .collect();
        for id in ids {
            if let Some(slot) = self.slot(id) {
                self.remove_slot(slot);
            }
        }
        Ok(missing)
    }

    fn remove_slot(&mut self, slot: usize) {
        self.active[slot] = false;
        self.live -= 1;
        if let Some(id) = self.records[slot].id() {
            self.ids.remove(id);
        }
    }

    /// Estimated heap bytes of the ID set, string IDs included; the records
    /// themselves are not counted.
    pub fn id_heap_bytes(&self) -> usize {
        self.ids.capacity() * (size_of::<RecordId>() + size_of::<usize>() + 1)
            + id_text_bytes(self.ids.keys())
            + self.active.capacity()
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
        if records.len() > self.max_records.saturating_sub(self.live) {
            return Err(Error::index_capacity(
                self.live.saturating_add(records.len()),
                self.max_records,
            ));
        }
        let mut batch = HashSet::new();
        for (index, record) in records.iter().enumerate() {
            let locate = |error: Error| error.with_record(index, record.id().cloned());
            if !record.is_prepared_with(&self.schema) {
                return Err(locate(Error::schema_mismatch()));
            }
            if let Some(id) = record.id()
                && (self.ids.contains_key(id) || !batch.insert(id))
            {
                return Err(locate(Error::duplicate_id()));
            }
        }
        for (offset, record) in records.iter().enumerate() {
            if let Some(id) = record.id() {
                self.ids.insert(id.clone(), start + offset);
            }
        }
        self.active.extend(std::iter::repeat_n(true, records.len()));
        self.live += records.len();
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
