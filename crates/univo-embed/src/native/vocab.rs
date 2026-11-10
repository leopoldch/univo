//! A string-to-id table read from `tokenizer.json`: every key in one byte
//! arena and an open-addressing index, about 30 bytes per token instead of
//! one heap string and hash-map entry each.

use std::hash::Hasher;

use rustc_hash::FxHasher;

/// Keys and ids in insertion order; a later duplicate key replaces the id
/// of the first, as a JSON map read into a hash map does.
#[derive(Debug, Default)]
pub(crate) struct Vocab {
    arena: Vec<u8>,
    /// `(start, end)` of each key in `arena`, and its id.
    entries: Vec<(u32, u32, u32)>,
    /// Entry index plus one; zero is an empty slot. Its length is a power
    /// of two at least twice the number of entries.
    slots: Vec<u32>,
    /// Byte length of the longest key: no longer substring can match.
    longest: usize,
}

impl Vocab {
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn longest(&self) -> usize {
        self.longest
    }

    pub(crate) fn get(&self, key: &[u8]) -> Option<u32> {
        if key.len() > self.longest || self.slots.is_empty() {
            return None;
        }
        let mask = self.slots.len() - 1;
        let mut slot = hash(key) & mask;
        loop {
            let entry = self.slots[slot];
            if entry == 0 {
                return None;
            }
            let (start, end, id) = self.entries[entry as usize - 1];
            if &self.arena[start as usize..end as usize] == key {
                return Some(id);
            }
            slot = (slot + 1) & mask;
        }
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.iter().map(|&(_, _, id)| id)
    }

    /// Keys starting with `prefix`, without it, with their ids.
    pub(crate) fn with_prefix(&self, prefix: &[u8]) -> Self {
        let mut stripped = Builder::default();
        for &(start, end, id) in &self.entries {
            if let Some(rest) = self.arena[start as usize..end as usize].strip_prefix(prefix) {
                stripped.push(rest, id);
            }
        }
        stripped.finish()
    }
}

/// Collects keys before the index is built.
#[derive(Debug, Default)]
pub(crate) struct Builder {
    arena: Vec<u8>,
    entries: Vec<(u32, u32, u32)>,
}

impl Builder {
    /// Appends `key`; the caller ensures the arena stays below 4 GiB.
    pub(crate) fn push(&mut self, key: &[u8], id: u32) {
        let start = self.arena.len();
        self.arena.extend_from_slice(key);
        self.push_last(start, id);
    }

    /// Records the bytes appended to the arena since `start` as one key.
    pub(crate) fn push_last(&mut self, start: usize, id: u32) {
        #[allow(clippy::cast_possible_truncation)]
        self.entries
            .push((start as u32, self.arena.len() as u32, id));
    }

    pub(crate) fn arena(&mut self) -> &mut Vec<u8> {
        &mut self.arena
    }

    pub(crate) fn finish(self) -> Vocab {
        let Self { arena, entries } = self;
        let capacity = (entries.len() * 2).max(2).next_power_of_two();
        let mask = capacity - 1;
        let mut vocab = Vocab {
            slots: vec![0; capacity],
            longest: 0,
            arena,
            entries: Vec::with_capacity(entries.len()),
        };
        for (start, end, id) in entries {
            let key = &vocab.arena[start as usize..end as usize];
            let mut slot = hash(key) & mask;
            loop {
                let entry = vocab.slots[slot];
                if entry == 0 {
                    vocab.entries.push((start, end, id));
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        vocab.slots[slot] = vocab.entries.len() as u32;
                    }
                    vocab.longest = vocab.longest.max(key.len());
                    break;
                }
                let existing = &mut vocab.entries[entry as usize - 1];
                if &vocab.arena[existing.0 as usize..existing.1 as usize] == key {
                    existing.2 = id;
                    break;
                }
                slot = (slot + 1) & mask;
            }
        }
        vocab
    }
}

fn hash(key: &[u8]) -> usize {
    let mut hasher = FxHasher::default();
    hasher.write(key);
    #[allow(clippy::cast_possible_truncation)]
    {
        hasher.finish() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_keys_and_keeps_the_last_duplicate() {
        let mut builder = Builder::default();
        for (key, id) in [("a", 1), ("##b", 2), ("abc", 3), ("a", 4), ("", 5)] {
            builder.push(key.as_bytes(), id);
        }
        let vocab = builder.finish();
        assert_eq!(vocab.len(), 4);
        assert_eq!(vocab.get(b"a"), Some(4));
        assert_eq!(vocab.get(b"abc"), Some(3));
        assert_eq!(vocab.get(b""), Some(5));
        assert_eq!(vocab.get(b"ab"), None);
        assert_eq!(vocab.get(b"abcd"), None);
        let continuing = vocab.with_prefix(b"##");
        assert_eq!(continuing.get(b"b"), Some(2));
        assert_eq!(continuing.get(b"a"), None);
    }

    #[test]
    fn an_empty_vocabulary_matches_nothing() {
        let vocab = Builder::default().finish();
        assert_eq!(vocab.get(b""), None);
        assert_eq!(vocab.get(b"a"), None);
    }
}
