//! Candidates for one text field by acronym: records whose text spells the
//! initials of the query's words, or whose words have the query's text as
//! initials.
//!
//! A text field reaches [`univo_core::ACRONYM_SIMILARITY`] against a text it is the
//! acronym of, whatever its lexical or semantic similarity. Every such pair
//! shares one key: the acronym on one side, equal to one of the initials of
//! the other side.

use std::collections::HashMap;

use univo_core::{PreparedRecord, Result, has_value, partition_fingerprint};

use crate::store::RecordStore;

/// Whether a key is a text's acronym or the initials of its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Role {
    Acronym,
    Initials,
}

/// Posting lists of the acronyms and initials of one text field, per
/// partition, keyed by the hashes the records were prepared with.
#[derive(Debug)]
pub(crate) struct AcronymIndex {
    field: usize,
    indexed: usize,
    postings: HashMap<(u64, Role, u32), Vec<u32>>,
}

/// The acronym and initials keys of a record's text, with their role.
fn keys(
    store: &RecordStore,
    record: &PreparedRecord,
    field: usize,
) -> impl Iterator<Item = (Role, u32)> {
    let prepared = &record.fields()[field];
    let keys = if has_value(&store.schema().fields()[field], prepared) {
        prepared.acronym_keys()
    } else {
        univo_core::AcronymKeys::default()
    };
    std::iter::once((Role::Acronym, keys.acronym))
        .chain(keys.initials.map(|initials| (Role::Initials, initials)))
        .filter(|&(_, key)| key != 0)
}

impl AcronymIndex {
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
            for (role, key) in keys(store, record, self.field) {
                self.postings
                    .entry((partition, role, key))
                    .or_default()
                    .push(slot);
            }
        }
        self.indexed = store.len();
        Ok(())
    }

    /// The posting lists of the records the query's text is the acronym of,
    /// and of those that are its acronym.
    pub(crate) fn lists(&self, store: &RecordStore, query: &PreparedRecord) -> Vec<&[u32]> {
        let partition = partition_fingerprint(query);
        keys(store, query, self.field)
            .filter_map(|(role, key)| {
                let other = match role {
                    Role::Acronym => Role::Initials,
                    Role::Initials => Role::Acronym,
                };
                self.postings
                    .get(&(partition, other, key))
                    .map(Vec::as_slice)
            })
            .collect()
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.postings
            .values()
            .map(|slots| slots.capacity() * 4)
            .sum::<usize>()
            + self.postings.capacity() * (size_of::<(u64, Role, u32)>() + size_of::<Vec<u32>>() + 1)
    }
}
