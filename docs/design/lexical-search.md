# Lexical candidates

`univo-engine` finds the records whose text is lexically similar to a query's, through an inverted index of character trigrams, without comparing every pair. One `LexicalIndex` covers one lexical or hybrid text field. [Search](search.md) combines it with the exact, semantic and value lanes.

## Contract

Records live in a `RecordStore`, bound to one schema, with unique typed IDs and an explicit record limit, both checked for a whole batch before any record is stored. The store is append-only: records are never reordered, replaced or removed, so an index built from it stays valid for the records it covers.
- **Building:** `LexicalIndex::new(&store, field)` indexes the records already stored.
- **Updating:** `index.update(&store)` indexes the records appended since the last update.
- **Store check:** `index.search(&store, query, threshold, max_entries)` checks, in constant time, that `store` is the store the index was built from and that no record was appended since the last update. Otherwise it returns `state.index_out_of_sync` and never reads mismatched records. The store has an identity and is deliberately not `Clone`, so two diverging copies cannot share it.

The search returns every record of the query's partition whose lexical similarity to the query is at least `threshold`, most similar first, then by slot.
- **Threshold:** it must satisfy `0 < threshold ≤ 1`; any other value, NaN included, is refused with `parameter.threshold` (`InvalidParameterError`, also a `ValueError`, in Python).
- **Similarity** is the scoring rule itself (`lexical_similarity`, see [scoring](../reference/scoring.md)): the larger of the edit similarity, for texts of at most 64 characters, and the trigram Jaccard index.
- **Completeness:** without a work limit, the result is exactly what comparing the query with every record would give.
- **Work limit:** `max_entries` caps the index entries read. When it stops the search, `truncated` is set and matches may be missing; the matches returned are still correct.
- **Counters:** `entries_read` and `candidates_verified` report the work done. A candidate rejected by its signature counts as verified, as it was before the signature existed.
- **Missing text:** blank, missing and null texts are neither indexed nor matched, as in scoring.
- **Partitions:** postings are keyed by partition, and every candidate's partition is compared by value, so a fingerprint collision cannot cross a partition.
- **Schema:** records prepared with another schema are refused by the store, and queries by the search, with `state.schema_mismatch`.

## How the search stays complete

Every candidate is verified with the scoring function, so the index only has to never miss one. With `A` the query's trigram set, padded with two spaces at each end:

- **Jaccard filter.** A Jaccard index of at least `t` implies at least `⌈t·|A|⌉` shared trigrams. A match therefore shares at least one of any `|A| − ⌈t·|A|⌉ + 1` trigrams of `A`; the search reads the rarest ones.
- **Edit filter.** An edit similarity of at least `t` bounds both the candidate's length (`t·m ≤ n ≤ m/t`) and the edit distance (`d ≤ (1−t)·max(m, n)`). The search cuts the padded query into `d + 1` disjoint segments of at least three characters. Each edit destroys at most one segment, so one segment survives intact in the match, with all its trigrams. Reading the rarest trigram of each segment is therefore enough.
- **Length scan.** When the query is too short for `d + 1` segments of three characters, the records of the lengths concerned are scanned instead.
- **Size filter.** Before a candidate's text is read, its length and trigram count, stored per record, must allow at least one of the two similarities to reach `t`.
- **Summary filter.** Each record also stores a 48-byte summary: a 128-bit signature, the bit of each of its trigrams' hash, and for texts of at most 64 characters its character counts in 32 hashed buckets. With `B` the candidate's set:
  - **Missing trigrams.** A bit of `A` absent from `B`'s signature stands for at least one trigram of `A` missing from `B`. Collisions can only hide missing trigrams, never invent them. The shared trigrams are therefore at most `min(|A| − m_A, |B| − m_B)`, with `m_A` and `m_B` the bits each side lacks, which bounds the Jaccard index from above.
  - **Edits.** One edit destroys at most three trigram positions, so `max(m_A, m_B)` missing trigrams need at least `⌈max(m_A, m_B) / 3⌉` edits (the q-gram bound, [Ukkonen, 1992](https://www.cs.helsinki.fi/u/ukkonen/TCS92.pdf)), and at least `|m − n|`.
  - **Characters.** One edit removes at most one character and adds at most one, so the edits are at least the larger of the characters each side has in excess (the bag distance). Counting by bucket can only merge differences, never create them. It is read only when the trigram bound has not already rejected the candidate. With the edits above, it bounds the edit similarity from above.
  - **Rejection.** A candidate is rejected only when both bounds are below `t`. They are computed with the comparator's own float arithmetic, so a score exactly at `t` passes. Otherwise the scoring function verifies the candidate, unchanged.
- **Records already read** are tracked in a bitmap over the index when the query's partition is a large share of it, in a hash set otherwise, so a search in a small partition does not allocate for the whole index.

## Cost

Synthetic company names of about 20 characters (two or three words of syllables and a number), 100,000 records in one partition, Apple M2, one thread:

| Threshold | Median | p95 | Records verified per query | Signatures only: median, p95 | No summary: median, p95 |
| --- | --- | --- | --- | --- | --- |
| 0.7 | 1.21 ms | 2.2–2.3 ms | 38,800 | 11.3–11.5 ms, 18.6 ms | 13.6–14.1 ms, 20.8 ms |
| 0.8 | 0.14 ms | 0.30 ms (budget 15 ms) | 8,000 | 0.23 ms, 0.75–0.77 ms | 2.9 ms, 6.3–6.5 ms |
| 0.9 | 18–21 µs | 38–45 µs | 620 | 18 µs, 37 µs | 0.23–0.27 ms, 0.58–0.66 ms |

- **Verified records.** They include those the summaries reject without scoring.
- **Thresholds.** At 0.7, an edit similarity allows many missing trigrams, so the trigram bounds alone reject little. The character counts reject most of what remains.

- **Build:** the index of 100,000 names builds in 35 ms, 27–30 ms without summaries (budget 50 ms), and takes about 14 MiB: 4 bytes per distinct trigram occurrence, one hash-map entry per distinct trigram and partition, and 16 bytes per record. The summaries add 48 bytes per record: the index reported grows from 14.4 to 20.4 MiB, with the vector's spare capacity.
- **Low thresholds are expensive by construction.** An edit similarity of 0.7 allows up to eight edits on 20 to 28 characters, so a complete search must examine a large part of the partition.
- **Small vocabularies cost more.** With names drawn from only twenty words, every trigram inside a word is frequent: 0.8 then takes 10 ms at p95.
- **Operating point:** U-010c chooses the threshold and the work limit by measuring the recall of duplicates against the exhaustive oracle.

## Validation

`crates/univo-engine/tests/lexical_search.rs`:
- **Oracle:**
  - every search equals a scan of every record for thresholds from 0.05 to 1;
  - lexical and hybrid fields, three partitions, typos, blank texts, Unicode, texts longer than 64 characters, and indexing in two batches.
- **Short strings:** `abc` and `xbz` share no trigram but reach an edit similarity of 1/3; they are found by the length scan.
- **Limits and schemas:**
  - truncation is reported, and its matches are a subset of the complete ones;
  - partitions, blank queries, foreign schemas and semantic fields are handled;
  - another store with the same records in another order, and a store with records appended since the last update, are refused; updating the index makes the new records searchable;
  - thresholds above 1, at or below 0, NaN and infinity are refused;
  - a small partition of a large index, searched through the hash set, still equals the scan.
- **Mutation checks:** reading one trigram instead of the rarest prefix, skipping the length scan, taking `d` segments instead of `d + 1`, and segments of two characters each make these tests fail.

Unit tests in `crates/univo-engine/src/lexical.rs` check the summary bounds against the scoring function:
- **Pairs.** Every pair of texts of up to four characters over `a`, `b`, a space and `é`, plus repetitions of 15 to 100 characters around the edit limit, and 160 names with typos, dropped and repeated words.
- **Thresholds.** 0.1, 0.5, 0.8, 0.9, 1, and each pair's score with the floats just above and below it.
- **Checks.** A rejected pair never reaches the threshold, and the bound rejects tens of thousands of pairs.
- **Mutation checks.** These tests fail when:
  - the edit bound uses `⌈m/2⌉` or `m_A + m_B`;
  - either comparison becomes strict;
  - one more trigram counts as missing;
  - the edit limit is applied at 64 characters instead of above it.
  - the bag distance adds the excess and missing characters instead of taking the larger, counts one more excess character per bucket, or counts characters twice;
  - the summary bound adds one edit to the bag distance.

  The oracle tests fail when a candidate is checked against another slot's summary.

`crates/univo-engine/tests/lexical_search_budget.rs` enforces the build and search budgets in release builds.
