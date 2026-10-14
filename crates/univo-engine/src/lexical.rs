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
//! every candidate is verified with the scoring function itself, unless its
//! signature proves it cannot reach the threshold:
//!
//! - Each record keeps a 128-bit signature, one bit per hashed trigram. A
//!   bit of `A` absent from `B` stands for at least one trigram of `A`
//!   missing from `B`; collisions only hide missing trigrams. This bounds
//!   the shared trigrams, hence the Jaccard index, from above.
//! - One edit destroys at most three trigram positions, so `m` trigrams
//!   missing from either side need at least `⌈m / 3⌉` edits (the q-gram
//!   bound, Ukkonen 1992).
//! - One edit removes at most one character and adds at most one, so at
//!   least as many edits are needed as characters one side has in excess
//!   (the bag distance). Each short text also keeps its character counts
//!   by hashed bucket; merging characters into buckets can only hide
//!   differences. With the trigram bound, this bounds the edit similarity
//!   from above.
//!
//! A candidate is rejected only when both bounds are below the threshold:
//! the search is exact, not approximate.

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
    /// Distinct records checked against the threshold: scored, or
    /// rejected by their signature.
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
    /// [`Summary`] of each record's text, by slot; empty without text.
    summaries: Vec<Summary>,
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
            summaries: Vec::new(),
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
            + self.summaries.capacity() * size_of::<Summary>()
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
            let mut summary = Summary::default();
            if let Some((text, trigrams)) = indexed_text(record, self.field) {
                summary = Summary::new(text, trigrams);
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
            self.summaries.push(summary);
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
        let mut result = self.search_from(store, query, threshold, max_entries, 0)?;
        result.matches.sort_by(|a, b| {
            b.similarity
                .total_cmp(&a.similarity)
                .then_with(|| a.slot.cmp(&b.slot))
        });
        Ok(result)
    }

    /// [`Self::search`] restricted to slots from `min_slot`, with matches in
    /// no particular order: all-pairs searches find each pair from its lower
    /// slot only. Lists hold slots in increasing order, so lower slots are
    /// skipped at once, yet still counted against `max_entries`: truncation
    /// does not depend on `min_slot`.
    pub(crate) fn search_from(
        &self,
        store: &RecordStore,
        query: &PreparedRecord,
        threshold: f32,
        max_entries: usize,
        min_slot: usize,
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

        let chars = text.chars().count();
        let filter = SizeFilter::new(chars, trigrams.len(), threshold);
        let summary = Summary::new(text, trigrams);
        let sketch = Sketch {
            chars,
            trigrams: trigrams.len(),
            summary: &summary,
        };
        let mut seen = Seen::new(
            self.partitions.len(),
            self.partition_sizes.get(&partition).copied().unwrap_or(0),
        );
        'lists: for slots in lists {
            debug_assert!(slots.is_sorted());
            let skipped = slots.partition_point(|&slot| (slot as usize) < min_slot);
            if skipped > max_entries - result.entries_read {
                result.entries_read = max_entries;
                result.truncated = true;
                break;
            }
            result.entries_read += skipped;
            for &slot in &slots[skipped..] {
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
                if !store.is_active(slot) {
                    continue;
                }
                let candidate = &records[slot];
                if candidate.partition() != query.partition() {
                    continue;
                }
                let (other_chars, other_trigrams) = self.sizes[slot];
                let other = Sketch {
                    chars: other_chars as usize,
                    trigrams: other_trigrams as usize,
                    summary: &self.summaries[slot],
                };
                // The size filter admits indexed texts only: a rejected
                // candidate counts as verified, as it would have been.
                if !sketch.may_reach(&other, threshold) {
                    result.candidates_verified += 1;
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

/// Buckets of the character counts a [`Summary`] keeps.
const BUCKETS: usize = 32;

/// What the bounds of [`Sketch::may_reach`] read from a text, 48 bytes.
#[derive(Debug, Clone, Copy, Default)]
struct Summary {
    /// The bit of each trigram's hash.
    signature: u128,
    /// Characters by hashed bucket, for texts of at most
    /// [`MAX_EDIT_CHARS`] characters, so a count never exceeds 64; zeros
    /// for longer texts, which edit distance does not compare.
    characters: [u8; BUCKETS],
}

impl Summary {
    fn new(text: &str, trigrams: &[u64]) -> Self {
        let signature = trigrams.iter().fold(0, |bits, &trigram| {
            bits | 1 << (trigram.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 57)
        });
        let mut characters = [0u8; BUCKETS];
        if text.chars().nth(MAX_EDIT_CHARS).is_none() {
            for character in text.chars() {
                characters[(u32::from(character).wrapping_mul(0x9E37_79B9) >> 27) as usize] += 1;
            }
        }
        Self {
            signature,
            characters,
        }
    }

    /// Edits needed at least between the texts: the larger of the
    /// characters each has in excess, bucket by bucket.
    fn bag_distance(&self, other: &Self) -> usize {
        let (mut excess, mut missing) = (0u32, 0u32);
        for (&a, &b) in self.characters.iter().zip(&other.characters) {
            excess += u32::from(a.saturating_sub(b));
            missing += u32::from(b.saturating_sub(a));
        }
        excess.max(missing) as usize
    }
}

/// A text as the bounds see it: its characters, distinct trigrams and
/// [`Summary`].
struct Sketch<'a> {
    chars: usize,
    trigrams: usize,
    summary: &'a Summary,
}

impl Sketch<'_> {
    /// Whether the texts may reach `threshold`: `false` only when upper
    /// bounds of both their Jaccard index and their edit similarity are
    /// below it (see the module documentation). The bounds use the
    /// comparator's own arithmetic, so a score at the threshold passes.
    fn may_reach(&self, other: &Self, threshold: f32) -> bool {
        let (signature, other_signature) = (self.summary.signature, other.summary.signature);
        let missing = (signature & !other_signature).count_ones() as usize;
        let other_missing = (other_signature & !signature).count_ones() as usize;
        let shared = self
            .trigrams
            .saturating_sub(missing)
            .min(other.trigrams.saturating_sub(other_missing));
        let union = self.trigrams + other.trigrams - shared;
        if union == 0 || ratio(shared, union) >= threshold {
            return true;
        }
        if self.chars > MAX_EDIT_CHARS || other.chars > MAX_EDIT_CHARS {
            return false;
        }
        let longest = self.chars.max(other.chars);
        let reaches = |edits| longest == 0 || 1.0 - ratio(edits, longest) >= threshold;
        let edits = missing
            .max(other_missing)
            .div_ceil(3)
            .max(self.chars.abs_diff(other.chars));
        // The bag distance reads two cache lines more: only when needed.
        reaches(edits) && reaches(edits.max(self.summary.bag_distance(other.summary)))
    }
}

/// `numerator / denominator` as `univo_core` computes similarities.
#[allow(clippy::cast_precision_loss)]
fn ratio(numerator: usize, denominator: usize) -> f32 {
    numerator as f32 / denominator as f32
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

#[cfg(test)]
mod tests {
    use super::*;
    use univo_core::{FieldPath, Map, Schema, Text, Value, prepare, trigrams};

    fn summary(text: &str) -> (Summary, Box<[u64]>) {
        let grams = trigrams(text);
        (Summary::new(text, &grams), grams)
    }

    fn sketch<'a>(text: &str, (summary, grams): &'a (Summary, Box<[u64]>)) -> Sketch<'a> {
        Sketch {
            chars: text.chars().count(),
            trigrams: grams.len(),
            summary,
        }
    }

    /// Every pair of `texts` at thresholds around its score: a rejected
    /// pair never reaches the threshold. Returns the rejections.
    fn check_signatures(texts: &[String]) -> usize {
        let summaries: Vec<_> = texts.iter().map(|text| summary(text)).collect();
        let mut rejected = 0;
        for (a, a_summary) in texts.iter().zip(&summaries) {
            let a_sketch = sketch(a, a_summary);
            for (b, b_summary) in texts.iter().zip(&summaries) {
                let b_sketch = sketch(b, b_summary);
                let (score, _) = lexical_similarity(a, &a_summary.1, b, &b_summary.1);
                for threshold in [
                    0.1,
                    0.5,
                    0.8,
                    0.9,
                    1.0,
                    score,
                    score.next_up(),
                    score.next_down(),
                ] {
                    if !(threshold > 0.0 && threshold <= 1.0) {
                        continue;
                    }
                    if !a_sketch.may_reach(&b_sketch, threshold) {
                        rejected += 1;
                        assert!(score < threshold, "{a:?} {b:?} {score} {threshold}");
                    }
                }
            }
        }
        rejected
    }

    #[test]
    fn signatures_never_reject_short_texts_that_reach_the_threshold() {
        // Every text of up to four characters over a small alphabet with a
        // space and a multibyte letter, and long repetitions around the
        // edit limit.
        let alphabet = ['a', 'b', ' ', 'é'];
        let mut texts = Vec::new();
        for length in 1..=4u32 {
            for mut value in 0..4usize.pow(length) {
                texts.push(
                    (0..length)
                        .map(|_| {
                            let character = alphabet[value % 4];
                            value /= 4;
                            character
                        })
                        .collect(),
                );
            }
        }
        for n in [15, 16, 21, 22, 63, 64, 65, 100] {
            texts.extend(["a".repeat(n), "é".repeat(n), "ab ".repeat(n / 3)]);
            texts.push(format!("{}b", "a".repeat(n - 1)));
        }
        assert!(check_signatures(&texts) > 10_000);
    }

    #[test]
    fn signatures_never_reject_names_that_reach_the_threshold() {
        // Names with typos, swapped and dropped words: the pairs search
        // meets, most of them below the threshold.
        let words = [
            "acme",
            "global",
            "institute",
            "of",
            "technology",
            "université",
            "paris",
            "saclay",
            "national",
            "research",
            "center",
            "société",
            "générale",
            "bank",
        ];
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            usize::try_from(state % bound as u64).unwrap()
        };
        let mut texts = Vec::new();
        for _ in 0..160 {
            let count = 1 + next(5);
            let mut name: Vec<char> = (0..count)
                .map(|_| words[next(words.len())])
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .collect();
            for _ in 0..next(3) {
                let at = next(name.len());
                match next(3) {
                    0 => {
                        name.remove(at);
                    }
                    1 => name.insert(at, 'x'),
                    _ => name[at] = 'e',
                }
            }
            texts.push(name.into_iter().collect());
        }
        assert!(check_signatures(&texts) > 50_000);
    }

    #[test]
    fn lower_slots_are_skipped_but_counted_against_the_work_limit() {
        let schema = Schema::builder()
            .text(FieldPath::parse("name").unwrap(), Text::default())
            .build()
            .unwrap();
        let names = [
            "acme corp",
            "acme corporation",
            "acme co",
            "acne corp",
            "acme",
        ];
        let records: Vec<_> = (0..60)
            .map(|i| {
                let name = format!("{} {}", names[i % names.len()], i % 7);
                let value = Map::new(vec![("name".into(), Value::Str(name))]).unwrap();
                prepare(&schema, &Value::Map(value)).unwrap()
            })
            .collect();
        let mut store = RecordStore::new(schema, usize::MAX);
        store.append(records.clone()).unwrap();
        let index = LexicalIndex::new(&store, 0).unwrap();
        let slots = |found: &LexicalSearch| {
            let mut slots: Vec<_> = found.matches.iter().map(|found| found.slot).collect();
            slots.sort_unstable();
            slots
        };
        let mut truncations = 0;
        for query in &records[..10] {
            let all = index
                .search_from(&store, query, 0.5, usize::MAX, 0)
                .unwrap();
            for max_entries in 0..=all.entries_read + 1 {
                let from_zero = index
                    .search_from(&store, query, 0.5, max_entries, 0)
                    .unwrap();
                truncations += usize::from(from_zero.truncated);
                for min_slot in [0, 1, 7, 30, 59, 60] {
                    let from = index
                        .search_from(&store, query, 0.5, max_entries, min_slot)
                        .unwrap();
                    assert_eq!(from.entries_read, from_zero.entries_read);
                    assert_eq!(from.truncated, from_zero.truncated);
                    let expected: Vec<_> = slots(&from_zero)
                        .into_iter()
                        .filter(|&slot| slot >= min_slot)
                        .collect();
                    assert_eq!(slots(&from), expected);
                }
            }
        }
        assert!(truncations > 100);
    }
}
