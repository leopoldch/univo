# Search

`SearchIndex` finds the records whose score against a query reaches a minimum, using the scoring of [pairwise scoring](../reference/scoring.md). It gathers candidates through one lane per field, then scores them, and returns exactly what scoring every record of the query's partition would return. Python access arrives with `Finder` (U-011).

```rust
use std::sync::Arc;
use univo_embed::Embedder;
use univo_engine::{SearchIndex, SearchOptions};

let model: Arc<dyn Embedder> = Arc::new(embedder);
let mut index = SearchIndex::new(schema, 1_000_000, Some(model))?;
index.add(records)?;
let result = index.search(&query, SearchOptions { min_score: 0.8, limit: 10, max_entries: 1 << 20 })?;
for found in &result.matches {
    println!("{} {:?}", found.slot, found.comparison.score);
}
```

## Why the result is complete

A record's score is a weighted mean of its field similarities, so a score of at least `s` needs at least one field with a similarity of at least `s`. Each lane finds every record that can reach `s` on its field:

| Field | Lane | Why it finds every candidate |
| --- | --- | --- |
| Text, lexical or hybrid | [Trigram index](lexical-search.md) | Complete for the lexical similarity at any threshold |
| Text, semantic | [Vector search](vector-search.md) over the query's partition | Scans every embedding |
| Exact scalar or list | Records with an equal value | The similarity is 1 for an equal value, 0 otherwise |
| Exact object | Records sharing one equal leaf, or both empty | A positive similarity needs an equal leaf |
| Any text, when `s ≤ 0.9` | Records whose text is the acronym of the query's words, or whose words the query's text abbreviates | Such a pair shares one key: the acronym on one side equals the initials of the other |

Two more rules complete the guarantee:
- **Rounding.** Lanes use `s·(1 − 10⁻⁶)`, so that rounding in the weighted mean never drops a record that reaches `s`. The margin is relative, so it stays positive even for the smallest positive minimum.
- **Exact records.** Records equal to the query on every field are always included, through the [exact index](exact-index.md), whatever the work limit.

Every candidate is then scored with `compare_with_vectors`:
- vetoed pairs and records of other partitions are never returned;
- records with the query's typed ID, or the query's own slot, are excluded.

## Contract

- **`search(query, options)`** embeds the query's semantic fields with the index's model. `search_slot(slot, options)` searches for a stored record and reuses its stored embeddings.
- **The oracle:** `search_exhaustive` and `search_slot_exhaustive` score every record of the query's partition. That costs `O(n)` per query and `O(n²)` for all pairs, without ever building a pairwise matrix.
- **Options:**
  - `min_score` must satisfy `0 < min_score ≤ 1` (`parameter.threshold`);
  - `limit` bounds the matches returned, while `total_matches` counts them all;
  - `max_entries` bounds the index entries each lexical, value or acronym lane reads. When a lane reaches it, `truncated` is set and matches may be missing. The exact lane and the semantic scan are not limited.
- **Order:** highest score first, then typed ID (strings before integers), then slot.
- **Memory:** every match is counted, but only the best `limit` are kept, in a bounded heap, and only those are explained: candidates are scored without per-field explanations. A group of 100,000 duplicates with `limit = 10` keeps ten explanations, not 100,000; ranking costs `O(M log limit)` for `M` matches.
- **Diagnostics:** `candidates` counts the records scored; `lanes` counts the candidates each lane proposed.
- **Adding records:** `add(records)` embeds the batch and makes every check before storing anything, so on error the index is unchanged. The checks are: schema, unique typed IDs, the record limit, and the model of the new embeddings against the stored ones (`state.model_mismatch` if the embedder changed).
- **Queries:** a query's embeddings are checked against the model of the stored ones in the same way. Slots beyond the records return `limit.index_capacity`.
- **Model:** a schema with semantic fields needs a model (`model.unavailable` otherwise).
- **Record limit:** at most 2³² records (`limit.index_capacity`).

## Cost

Standard workload:
- 100,000 records: ID, tenant, a name of about 20 characters, a semantic description of 30 words;
- one record in ten is a near-duplicate of the previous one;
- synthetic embeddings of 256 dimensions;
- search of a stored record at a minimum score of 0.8, `limit = 10`, Apple M2, one thread.

Each scenario runs five independent repetitions (index rebuilt each time) of 1,000 queries after a warmup. The figures are the median repetition.

| Scenario | Median | p95 (range over 5) | p99 | Budget (p95) |
| --- | --- | --- | --- | --- |
| 1 partition | 5.9 ms | 9.9 ms (9.8–10.1) | 11.9 ms | 25 ms |
| 10 partitions | 1.7 ms | 2.2 ms (2.2–2.2) | 2.5 ms | 25 ms |
| 1 partition, queries from a group of 10,000 identical records | 7.6 ms | 8.0 ms (8.0–8.1) | 8.3 ms | 20 ms |

- **Group scenario.** Each query scores the 9,999 other members, all matches, and keeps ten.
- **Specification.** It allows 50 ms at the p95 with four threads.
- **Where the time goes.** Most of it goes to the semantic scan (about 2.5 ms per 100,000 vectors) and the lexical lane.

**Build and memory.** Building the indexes for 100,000 records takes about 0.18 s with synthetic embeddings; encoding with the default model adds about 47 µs per distinct description on one thread. The estimated heap of the indexes is about 115 MiB: every lane, the exact index and the ID set, most of it the 98 MiB of embeddings. Records and the model are not counted.

**Lower minimum scores cost more.** The lexical lane's cost rises sharply below 0.8 ([lexical candidates](lexical-search.md)). The semantic scan stays linear in the partition until an approximate index replaces it (U-016).

## Validation

- `crates/univo-engine/tests/search_oracle.rs`: search equals the exhaustive oracle, matches and counts, on stored records and new queries.
  - **Data:** minimum scores from 0.2 to 1; a partition; typed IDs; a hybrid name with typos and with acronyms of its words; a hostname with blank values; a discriminant SKU; nested objects that can be empty or null; a semantic description, some without known tokens; indexing in two batches.
  - **Mutation checks:** removing the lexical, semantic, value or acronym lane, nested object leaves, or the empty-object key makes the tests fail. So does reading the acronym lane only below 0.5, looking up a key under its own role, ignoring partitions in its keys, or omitting it for semantic fields.
  - **Also covered:**
    - ranking and limits, including a group of a thousand identical records with limits from 0 to 500;
    - work limits that keep exact matches and report truncation;
    - invalid minimum scores, and minimum scores as small as `10⁻⁴⁰` with a subnormal cosine;
    - foreign schemas, missing models, oversized limits, and slots up to `usize::MAX`;
    - failed batches, and an embedder whose descriptor changes, leaving the index unchanged.
  - A reversed ID order in the bounded heap makes the tests fail.
- `crates/univo-engine/tests/search_budget.rs` enforces the budget in release builds.

Semantic fields can use optional [HNSW routing](../guide/ann.md). The dense default
and exhaustive oracle remain complete within their declared limits. Reports
using a graph expose `approximate` even without observed saturation; the
exact/lexical union and core reranking remain. Snapshot reconstruction and
compaction can change approximate recall without changing logical generation.
