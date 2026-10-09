# Vector search (U-010)

`univo-engine` keeps the embeddings of the semantic text fields of prepared records and searches them exhaustively. This exhaustive search is the reference that any approximate index must be measured against ([specs/03-matching.md §12](../specs/03-matching.md)), and a supported mode within its cost. [Search](search.md) uses it as the semantic lane, over the query's partition.

## Embedding records

`embed_records(schema, records, embedder)` embeds every field declared `semantic`, with one `encode_batch` call per field. Hybrid fields are scored lexically and are not embedded ([ADR 0009](adr/0009-semantic-evidence-in-scoring.md)).

- The text embedded is the normalized one: NFC and, unless the field disables it, case folded. Two spellings that normalize alike are one text.
- Identical texts in a batch are encoded once and stored once: records point to a shared row.
- A missing or blank text, or one without any token the model knows, has no embedding. Pairwise scoring then treats the field as giving no evidence.
- `RecordVectors` keeps the schema and the model that produced it. `extend` appends a later batch only if both are the same, including the model's revision, files and dimension. Otherwise it leaves the vectors unchanged and returns `state.schema_mismatch` or `state.model_mismatch`.
- Records prepared with another schema are refused with `state.schema_mismatch`. An embedder that returns another number of rows, or another dimension than it declares, is reported as `model.corrupt`.
- `RecordVectors::record(slot)` returns the embeddings of one record in schema order, as `compare_with_vectors` takes them.

Each field stores its rows in one contiguous `f32` buffer, plus 8 bytes per record for its row number: 1 KiB per distinct text at 256 dimensions, about 98 MiB for 100,000 distinct texts.

## Searching

Both functions take the embeddings of one field, a query vector and the candidate slots. The caller chooses the candidates, for instance one partition's records without the query itself.

- `top_k(vectors, query, candidates, k, key)` returns the `k` most similar candidates. It uses `O(k)` memory and never builds a pairwise matrix.
- `within(vectors, query, candidates, threshold, key)` returns every candidate at or above `threshold`. This is the complete answer, so its size grows with the number of matches.

Both functions follow the same rules:
- **Similarity** is the one used in scoring: the cosine clamped to `[0, 1]`.
- **Order** is most similar first. Ties are broken by `key(slot)`, then by slot, smallest first, so results never depend on the order of the candidates. `top_k(k)` is exactly the first `k` results of `within(0)`.
- **Skipped candidates:** those without an embedding, and every candidate when the query has a different dimension.

## Cost

A query reads every candidate's row once: `O(n·d)` operations, and `O(n²·d)` for all pairs of a set of `n` records.

The dot product sums eight independent lanes:
- the compiler vectorizes it with the instructions every supported CPU has (SSE2, NEON), with no runtime feature detection;
- it avoids fused multiply-add, which is a slow library call on CPUs without it;
- the summation order is fixed, so results are identical from run to run.

| Measure (Apple M2, one thread) | Result | Budget |
| --- | --- | --- |
| Top-10 over 100,000 vectors of 256 dimensions, median of 25 queries | 2.5 ms, about 10 billion multiply-adds per second | 5 ms |

## Validation

- `crates/univo-engine/tests/dense_search.rs` covers the search results:
  - `top_k` and `within` against a full sort, with frequent equal similarities, two tie-break keys, candidate subsets, `k` from 0 to beyond the candidates, and several thresholds;
  - independence from candidate order, including equal similarities and equal keys;
  - queries of another dimension.
- The same file covers embedding:
  - normalization-aware deduplication and shared rows;
  - missing texts and texts without known tokens;
  - only semantic fields embedded;
  - batches appended, and batches of another schema, model or dimension refused unchanged;
  - records of another schema, and embedders returning too few rows or rows of another dimension.
- `crates/univo-engine/tests/vector_search_budget.rs` enforces the budget in release builds.
