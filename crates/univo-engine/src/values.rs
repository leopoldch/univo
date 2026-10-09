//! Candidates for one exact field: records whose value equals the query's
//! or, for objects compared key by key, that share one equal leaf.
//!
//! An exact field's similarity is 1 for equal values and 0 otherwise; an
//! object's is the share of equal leaves among the shared ones. A positive
//! similarity therefore needs an equal value or an equal leaf, and these
//! postings find every record that can have one.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use univo_core::{
    FieldState, NormalizedValue, PreparedRecord, Result, has_value, partition_fingerprint,
    value_fingerprint,
};

use crate::store::RecordStore;

/// Key of two empty objects, which are equal values without leaves.
const EMPTY_OBJECT: u64 = 0x9e37_79b9_7f4a_7c15;

/// Posting lists of the values of one exact field, per partition.
#[derive(Debug)]
pub(crate) struct ValueIndex {
    field: usize,
    indexed: usize,
    postings: HashMap<(u64, u64), Vec<u32>>,
}

/// The keys a record's field value is found under.
fn keys(store: &RecordStore, record: &PreparedRecord, field: usize) -> Vec<u64> {
    let prepared = &record.fields()[field];
    if !has_value(&store.schema().fields()[field], prepared) {
        return Vec::new();
    }
    match &prepared.state {
        FieldState::Exact(NormalizedValue::Map(entries)) if entries.is_empty() => {
            vec![EMPTY_OBJECT]
        }
        FieldState::Exact(NormalizedValue::Map(entries)) => {
            let mut keys = Vec::new();
            leaves(entries, &mut Vec::new(), &mut keys);
            keys.sort_unstable();
            keys.dedup();
            keys
        }
        FieldState::Exact(value) => vec![value_fingerprint(value)],
        _ => Vec::new(),
    }
}

/// Keys of the leaves of an object, as pairwise scoring compares them:
/// null values are unknown, non-empty nested objects are descended into,
/// and any other value, an empty object included, is a leaf at its path.
fn leaves<'a>(
    entries: &'a [(Box<str>, NormalizedValue)],
    path: &mut Vec<&'a str>,
    keys: &mut Vec<u64>,
) {
    for (key, value) in entries {
        path.push(key);
        match value {
            NormalizedValue::Null => {}
            NormalizedValue::Map(nested) if !nested.is_empty() => leaves(nested, path, keys),
            leaf => {
                let mut hasher = DefaultHasher::new();
                path.hash(&mut hasher);
                hasher.write_u64(value_fingerprint(leaf));
                keys.push(hasher.finish());
            }
        }
        path.pop();
    }
}

impl ValueIndex {
    pub(crate) fn new(field: usize) -> Self {
        Self {
            field,
            indexed: 0,
            postings: HashMap::new(),
        }
    }

    /// Indexes the records appended to `store` since the last update.
    pub(crate) fn update(&mut self, store: &RecordStore) -> Result<()> {
        for (slot, record) in store.records().iter().enumerate().skip(self.indexed) {
            let slot = u32::try_from(slot)
                .map_err(|_| univo_core::Error::index_capacity(slot + 1, u32::MAX as usize))?;
            let partition = partition_fingerprint(record);
            for key in keys(store, record, self.field) {
                self.postings
                    .entry((partition, key))
                    .or_default()
                    .push(slot);
            }
        }
        self.indexed = store.len();
        Ok(())
    }

    /// The posting lists of the query's value, rarest first.
    pub(crate) fn lists(&self, store: &RecordStore, query: &PreparedRecord) -> Vec<&[u32]> {
        let partition = partition_fingerprint(query);
        let mut lists: Vec<&[u32]> = keys(store, query, self.field)
            .into_iter()
            .filter_map(|key| self.postings.get(&(partition, key)).map(Vec::as_slice))
            .collect();
        lists.sort_unstable_by_key(|slots| slots.len());
        lists
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.postings
            .values()
            .map(|slots| slots.capacity() * 4)
            .sum::<usize>()
            + self.postings.capacity() * (size_of::<(u64, u64)>() + size_of::<Vec<u32>>() + 1)
    }
}
