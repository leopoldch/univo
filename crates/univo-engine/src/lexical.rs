//! Lexical candidates for one text field: every record whose lexical
//! similarity to a query reaches a threshold, found through an inverted
//! index of character trigrams instead of comparing every pair.
//!
//! The search is complete for the field's similarity, the larger of the
//! edit similarity and the trigram Jaccard index (`docs/scoring.md`), unless
//! the work limit truncates it, which the result reports. Two filters give
//! the guarantee, where `A` is the query's trigram set:
//!
//! - Jaccard index `≥ t` implies sharing at least `⌈t·|A|⌉` trigrams, so a
//!   match shares one of any `|A| - ⌈t·|A|⌉ + 1` trigrams of `A`.
//! - Edit distance `≤ k`: cut the padded query into `k + 1` disjoint
//!   segments of at least three characters. Each edit destroys at most one
//!   segment, so one survives intact in the match, with all its trigrams:
//!   the match has one of the trigrams chosen in each segment. When the
//!   query is too short for `k + 1` such segments, the records of the
//!   lengths concerned are scanned instead.
//!
//! The rarest trigrams are chosen, so frequent ones are rarely read, and
//! every candidate is verified with the scoring function itself.

use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;

use univo_core::{
    Error, FieldKind, FieldState, MAX_EDIT_CHARS, PreparedRecord, Result, Signal, TextMode,
    lexical_similarity, partition_fingerprint, positional_trigrams,
};

use crate::store::RecordStore;

/// A record whose text is similar enough to the query's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LexicalMatch {
    pub slot: usize,
    pub similarity: f32,
    pub signal: Signal,
}

/// Matches with the work done to find them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LexicalSearch {
    /// Most similar first, then by slot.
    pub matches: Vec<LexicalMatch>,
    /// Posting and length-bucket entries read.
    pub entries_read: usize,
    /// Distinct records whose similarity was computed.
    pub candidates_verified: usize,
    /// The work limit stopped the search: matches may be missing.
    pub truncated: bool,
}

/// An inverted index of the trigrams of one lexical or hybrid text field,
/// per partition, covering the first records of one [`RecordStore`].
#[derive(Debug)]
pub struct LexicalIndex {
    store: u64,
    field: usize,
    /// Partition key of each record, by slot.
    partitions: Vec<u64>,
    /// Indexed records of each partition.
    partition_sizes: HashMap<u64, usize>,
    /// Characters and distinct trigrams of each record's text, by slot;
    /// zero trigrams when the record has no indexed text.
    sizes: Vec<(u32, u32)>,
    postings: HashMap<(u64, u64), Vec<u32>>,
    /// Records of each partition by character count, for texts short
    /// enough to be compared by edit distance.
    lengths: HashMap<(u64, u32), Vec<u32>>,
}

/// The normalized text and trigrams of a field that gives lexical evidence.
fn indexed_text(record: &PreparedRecord, field: usize) -> Option<(&str, &[u64])> {
    let prepared = &record.fields()[field];
    match &prepared.state {
        FieldState::Text(text) if !text.trim().is_empty() => Some((text, prepared.trigrams()?)),
        _ => None,
    }
}

fn slot_number(slot: usize) -> Result<u32> {
    u32::try_from(slot).map_err(|_| Error::index_capacity(slot + 1, u32::MAX as usize))
}

impl LexicalIndex {
    /// An index of field `field` of `store`'s schema, which must be a
    /// lexical or hybrid text field, covering the records already stored.
    ///
    /// # Errors
    ///
    /// Returns `state.schema_mismatch` for any other field.
    pub fn new(store: &RecordStore, field: usize) -> Result<Self> {
        match store.schema().fields().get(field).map(|field| &field.kind) {
            Some(FieldKind::Text(text)) if text.mode != TextMode::Semantic => {}
            _ => return Err(Error::schema_mismatch()),
        }
        let mut index = Self {
            store: store.id(),
            field,
            partitions: Vec::new(),
            partition_sizes: HashMap::new(),
            sizes: Vec::new(),
            postings: HashMap::new(),
            lengths: HashMap::new(),
        };
        index.update(store)?;
        Ok(index)
    }

    /// Number of records, indexed or not.
    pub fn len(&self) -> usize {
        self.partitions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.partitions.is_empty()
    }

    /// Estimated heap bytes: posting entries, hash-map entries, the
    /// per-partition counts and the per-record tables.
    pub fn heap_bytes(&self) -> usize {
        let lists = |map: &HashMap<(u64, u64), Vec<u32>>| -> usize {
            map.values()
                .map(|slots| slots.capacity() * 4)
                .sum::<usize>()
                + map.capacity() * (size_of::<(u64, u64)>() + size_of::<Vec<u32>>() + 1)
        };
        let lengths: usize = self
            .lengths
            .values()
            .map(|slots| slots.capacity() * 4)
            .sum::<usize>()
            + self.lengths.capacity() * (size_of::<(u64, u32)>() + size_of::<Vec<u32>>() + 1);
        lists(&self.postings)
            + lengths
            + self.partition_sizes.capacity() * (size_of::<(u64, usize)>() + 1)
            + self.partitions.capacity() * size_of::<u64>()
            + self.sizes.capacity() * size_of::<(u32, u32)>()
    }

    /// Indexes the records appended to `store` since the last update.
    ///
    /// # Errors
    ///
    /// Returns `state.index_out_of_sync` for another store, and
    /// `limit.index_capacity` beyond 2^32 records.
    pub fn update(&mut self, store: &RecordStore) -> Result<()> {
        if store.id() != self.store {
            return Err(Error::index_out_of_sync(self.len(), store.len(), false));
        }
        slot_number(store.len())?;
        let records = &store.records()[self.partitions.len()..];
        for record in records {
            let slot = slot_number(self.partitions.len())?;
            let partition = partition_fingerprint(record);
            let mut size = (0, 0);
            if let Some((text, trigrams)) = indexed_text(record, self.field) {
                for trigram in trigrams {
                    self.postings
                        .entry((partition, *trigram))
                        .or_default()
                        .push(slot);
                }
                let chars = text.chars().count();
                if let Some(chars) = u32::try_from(chars)
                    .ok()
                    .filter(|_| chars <= MAX_EDIT_CHARS)
                {
                    self.lengths
                        .entry((partition, chars))
                        .or_default()
                        .push(slot);
                }
                size = (
                    u32::try_from(chars).unwrap_or(u32::MAX),
                    u32::try_from(trigrams.len()).unwrap_or(u32::MAX),
                );
            }
            self.partitions.push(partition);
            *self.partition_sizes.entry(partition).or_default() += 1;
            self.sizes.push(size);
        }
        Ok(())
    }

    /// Every record of the query's partition whose lexical similarity to
    /// `query` is at least `threshold`, reading at most `max_entries` index
    /// entries. `store` must be the store the index covers, updated.
    ///
    /// # Errors
    ///
    /// Returns `parameter.threshold` unless `0 < threshold ≤ 1`,
    /// `state.index_out_of_sync` for another store or records appended
    /// since the last update, and `state.schema_mismatch` when `query` was
    /// prepared with another schema.
    pub fn search(
        &self,
        store: &RecordStore,
        query: &PreparedRecord,
        threshold: f32,
        max_entries: usize,
    ) -> Result<LexicalSearch> {
        if !(threshold > 0.0 && threshold <= 1.0) {
            return Err(Error::invalid_threshold(threshold));
        }
        if store.id() != self.store || store.len() != self.len() {
            return Err(Error::index_out_of_sync(
                self.len(),
                store.len(),
                store.id() == self.store,
            ));
        }
        if !query.is_prepared_with(store.schema()) {
            return Err(Error::schema_mismatch());
        }
        let records = store.records();
        let mut result = LexicalSearch::default();
        let Some((text, trigrams)) = indexed_text(query, self.field) else {
            return Ok(result);
        };
        let partition = partition_fingerprint(query);
        let lists = self.plan(partition, text, trigrams, threshold);

        let filter = SizeFilter::new(text.chars().count(), trigrams.len(), threshold);
        let mut seen = Seen::new(
            self.partitions.len(),
            self.partition_sizes.get(&partition).copied().unwrap_or(0),
        );
        'lists: for slots in lists {
            for &slot in slots {
                if result.entries_read == max_entries {
                    result.truncated = true;
                    break 'lists;
                }
                result.entries_read += 1;
                if !seen.insert(slot) {
                    continue;
                }
                let slot = slot as usize;
                if self.partitions[slot] != partition || !filter.admits(self.sizes[slot]) {
                    continue;
                }
                let candidate = &records[slot];
                if candidate.partition() != query.partition() {
                    continue;
                }
                let Some((other, other_trigrams)) = indexed_text(candidate, self.field) else {
                    continue;
                };
                result.candidates_verified += 1;
                let (similarity, signal) =
                    lexical_similarity(text, trigrams, other, other_trigrams);
                if similarity >= threshold {
                    result.matches.push(LexicalMatch {
                        slot,
                        similarity,
                        signal,
                    });
                }
            }
        }
        result.matches.sort_by(|a, b| {
            b.similarity
                .total_cmp(&a.similarity)
                .then_with(|| a.slot.cmp(&b.slot))
        });
        Ok(result)
    }

    fn postings(&self, partition: u64, trigram: u64) -> &[u32] {
        self.postings
            .get(&(partition, trigram))
            .map_or(&[], Vec::as_slice)
    }

    /// The posting lists and length buckets one search must read, rarest
    /// first.
    fn plan(&self, partition: u64, text: &str, trigrams: &[u64], threshold: f32) -> Vec<&[u32]> {
        let frequency = |trigram: u64| (self.postings(partition, trigram).len(), trigram);
        let mut chosen: Vec<u64> = Vec::new();

        // Jaccard filter: the rarest trigrams of the query.
        let size = trigrams.len();
        let shared = ceil_count(f64::from(threshold) * exact(size) - 1e-4).max(1);
        let mut rarest = trigrams.to_vec();
        rarest.sort_unstable_by_key(|trigram| frequency(*trigram));
        chosen.extend(&rarest[..(size.saturating_sub(shared) + 1).min(size)]);

        // Edit filter, for each edit distance the candidate lengths allow.
        let mut scanned = Vec::new();
        let chars = text.chars().count();
        if chars <= MAX_EDIT_CHARS {
            let positions = positional_trigrams(text);
            let padded = chars + 4;
            for (distance, lengths) in edit_bands(chars, threshold) {
                let segments = distance + 1;
                if 3 * segments > padded {
                    scanned.push(lengths);
                    continue;
                }
                let (base, longer) = (padded / segments, padded % segments);
                let mut start = 0;
                for segment in 0..segments {
                    let end = start + base + usize::from(segment < longer);
                    // Trigrams starting at `start..=end - 3` lie inside.
                    let best = positions[start..=end - 3]
                        .iter()
                        .copied()
                        .min_by_key(|trigram| frequency(*trigram));
                    chosen.extend(best);
                    start = end;
                }
            }
        }

        chosen.sort_unstable_by_key(|trigram| frequency(*trigram));
        chosen.dedup();
        let mut lists: Vec<&[u32]> = chosen
            .into_iter()
            .map(|trigram| self.postings(partition, trigram))
            .collect();
        for lengths in scanned {
            lists.extend(lengths.filter_map(|length| {
                let length = u32::try_from(length).ok()?;
                self.lengths.get(&(partition, length)).map(Vec::as_slice)
            }));
        }
        lists
    }
}

/// For a query of `chars` characters, the lengths a text can have while its
/// edit similarity reaches `threshold`, grouped by the largest edit
/// distance each allows: `1 - d / max(m, n) ≥ t` gives `t·m ≤ n ≤ m / t`
/// and `d ≤ (1 - t)·max(m, n)`. Bounds are widened slightly against
/// rounding.
fn edit_bands(chars: usize, threshold: f32) -> Vec<(usize, RangeInclusive<usize>)> {
    let (length, threshold) = (exact(chars), f64::from(threshold));
    let shortest = ceil_count(threshold * length - 1e-3).max(1);
    let longest = floor_count(length / threshold + 1e-3)
        .min(MAX_EDIT_CHARS)
        .max(chars);
    let mut bands: Vec<(usize, RangeInclusive<usize>)> = Vec::new();
    for other in shortest..=longest {
        let distance = floor_count((1.0 - threshold) * exact(chars.max(other)) + 1e-3);
        match bands.last_mut() {
            Some((last, lengths)) if *last == distance => *lengths = *lengths.start()..=other,
            _ => bands.push((distance, other..=other)),
        }
    }
    bands
}

/// Records already read during one search: a bitmap over every slot when
/// the partition is a large share of the index, a hash set otherwise, so
/// that a search in a small partition does not allocate for the whole index.
enum Seen {
    Bitmap(Vec<u64>),
    Set(HashSet<u32>),
}

impl Seen {
    fn new(slots: usize, partition: usize) -> Self {
        if partition.saturating_mul(32) >= slots {
            Self::Bitmap(vec![0; slots.div_ceil(64)])
        } else {
            Self::Set(HashSet::with_capacity(partition.min(1024)))
        }
    }

    /// Records `slot`, and returns whether it was new.
    fn insert(&mut self, slot: u32) -> bool {
        match self {
            Self::Bitmap(words) => {
                let (word, bit) = (slot as usize / 64, 1u64 << (slot % 64));
                let new = words[word] & bit == 0;
                words[word] |= bit;
                new
            }
            Self::Set(set) => set.insert(slot),
        }
    }
}

/// Sizes a candidate needs before its text is read: a Jaccard index of `t`
/// needs `t·|A| ≤ |B| ≤ |A| / t`, and an edit similarity of `t` needs
/// `t·m ≤ n ≤ m / t` characters, both texts within the edit limit.
struct SizeFilter {
    trigrams: RangeInclusive<usize>,
    chars: Option<RangeInclusive<usize>>,
}

impl SizeFilter {
    fn new(chars: usize, trigrams: usize, threshold: f32) -> Self {
        let threshold = f64::from(threshold);
        let range = |size: usize| {
            ceil_count(threshold * exact(size) - 1e-3)..=floor_count(exact(size) / threshold + 1e-3)
        };
        Self {
            trigrams: range(trigrams),
            chars: (chars <= MAX_EDIT_CHARS).then(|| {
                let lengths = range(chars);
                *lengths.start()..=(*lengths.end()).min(MAX_EDIT_CHARS)
            }),
        }
    }

    fn admits(&self, (chars, trigrams): (u32, u32)) -> bool {
        let (chars, trigrams) = (chars as usize, trigrams as usize);
        trigrams > 0
            && (self.trigrams.contains(&trigrams)
                || self
                    .chars
                    .as_ref()
                    .is_some_and(|lengths| lengths.contains(&chars)))
    }
}

/// A count as a float; counts here are character and trigram numbers, far
/// below 2^52, so the conversion is exact.
#[allow(clippy::cast_precision_loss)]
fn exact(count: usize) -> f64 {
    count as f64
}

/// The smallest count at least `value`, 0 for negative values.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ceil_count(value: f64) -> usize {
    value.ceil().max(0.0) as usize
}

/// The largest count at most `value`, 0 for negative values.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn floor_count(value: f64) -> usize {
    value.floor().max(0.0) as usize
}
