# Lexical candidates (U-010)

`univo-engine` finds the records whose text is lexically similar to a query's, through an inverted index of character trigrams, without comparing every pair. One `LexicalIndex` covers one lexical or hybrid text field. Combining it with exact and semantic candidates, and choosing thresholds and limits, is the candidate union of U-010's next step.

## Contract

`index.search(records, query, threshold, max_entries)` returns every record of the query's partition whose lexical similarity to the query is at least `threshold`, most similar first, then by slot.
- **Similarity** is the scoring rule itself (`lexical_similarity`, see [scoring](scoring.md)): the larger of the edit similarity, for texts of at most 64 characters, and the trigram Jaccard index.
- **Completeness:** without a work limit, the result is exactly what comparing the query with every record would give.
- **Work limit:** `max_entries` caps the index entries read. When it stops the search, `truncated` is set and matches may be missing; the matches returned are still correct.
- **Counters:** `entries_read` and `candidates_verified` report the work done.
- **Missing text:** blank, missing and null texts are neither indexed nor matched, as in scoring.
- **Partitions:** postings are keyed by partition, and every candidate's partition is compared by value, so a fingerprint collision cannot cross a partition.
- **Schema:** records and queries prepared with another schema are refused with `state.schema_mismatch`.

## How the search stays complete

Every candidate is verified with the scoring function, so the index only has to never miss one. With `A` the query's trigram set, padded with two spaces at each end:

- **Jaccard filter.** A Jaccard index of at least `t` implies at least `⌈t·|A|⌉` shared trigrams. A match therefore shares at least one of any `|A| − ⌈t·|A|⌉ + 1` trigrams of `A`; the search reads the rarest ones.
- **Edit filter.** An edit similarity of at least `t` bounds both the candidate's length (`t·m ≤ n ≤ m/t`) and the edit distance (`d ≤ (1−t)·max(m, n)`). The search cuts the padded query into `d + 1` disjoint segments of at least three characters. Each edit destroys at most one segment, so one segment survives intact in the match, with all its trigrams. Reading the rarest trigram of each segment is therefore enough.
- **Length scan.** When the query is too short for `d + 1` segments of three characters, the records of the lengths concerned are scanned instead.
- **Size filter.** Before a candidate's text is read, its length and trigram count, stored per record, must allow at least one of the two similarities to reach `t`.

## Cost

Synthetic company names of about 20 characters (two or three words of syllables and a number), 100,000 records in one partition, Apple M2, one thread:

| Threshold | Median | p95 | Records verified per query |
| --- | --- | --- | --- |
| 0.7 | 13.8 ms | 20.3 ms | 38,800 |
| 0.8 | 2.9 ms | 6.5 ms (budget 15 ms) | 8,000 |
| 0.9 | 0.24 ms | 0.63 ms | 620 |

- **Build:** the index of 100,000 names builds in 25 ms (budget 50 ms) and takes about 14 MiB: 4 bytes per distinct trigram occurrence, one hash-map entry per distinct trigram and partition, and 16 bytes per record.
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
  - partitions, blank queries, foreign schemas and semantic fields are handled.
- **Mutation checks:** reading one trigram instead of the rarest prefix, skipping the length scan, taking `d` segments instead of `d + 1`, and segments of two characters each make these tests fail.

`crates/univo-engine/tests/lexical_search_budget.rs` enforces the build and search budgets in release builds.
