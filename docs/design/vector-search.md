# Vector search

`univo-engine` keeps the embeddings of the semantic text fields of prepared records and searches them exhaustively. This exhaustive search is the reference that any approximate index must be measured against, and a supported mode within its cost. [Search](search.md) uses it as the semantic lane, over the query's partition.

## Embedding records

`embed_records(schema, records, embedder)` embeds every field declared `semantic`, with one `encode_batch` call per field. Hybrid fields are scored lexically and are not embedded.

- The text embedded is the normalized one: NFC and, unless the field disables it, case folded. Two spellings that normalize alike are one text.
- Identical texts in a batch are encoded once and stored once: records point to a shared row.
- A missing or blank text, or one without any token the model knows, has no embedding. Pairwise scoring then treats the field as giving no evidence.
- `RecordVectors` keeps the schema and the model that produced it. `extend` appends a later batch only if both are the same, including the model's revision, files and dimension. Otherwise it leaves the vectors unchanged and returns `state.schema_mismatch` or `state.model_mismatch`.
- Records prepared with another schema are refused with `state.schema_mismatch`. An embedder that returns another number of rows, or another dimension than it declares, is reported as `model.corrupt`.
- `RecordVectors::record(slot)` returns the embeddings of one record in schema order, as `compare_with_vectors` takes them.

Each field stores its rows in one contiguous `f32` buffer, an int8 copy of each row with 12 bytes of bounds (see [Pruning](#pruning)), and 8 bytes per record for its row number. At the default model's 128 dimensions, a distinct text takes 652 bytes, 512 of them for the `f32` row: about 63 MiB for 100,000 distinct texts. The int8 copy is derived from the rows; snapshots do not store it, and loading recomputes it.

## Searching

Both functions take the embeddings of one field, a query vector and the candidate slots. The caller chooses the candidates, for instance one partition's records without the query itself.

- `top_k(vectors, query, candidates, k, key)` returns the `k` most similar candidates. It uses `O(k)` memory and never builds a pairwise matrix.
- `within(vectors, query, candidates, threshold, key)` returns every candidate at or above `threshold`. This is the complete answer, so its size grows with the number of matches.

Both functions follow the same rules:
- **Similarity** is the one used in scoring: the cosine clamped to `[0, 1]`.
- **Order** is most similar first. Ties are broken by `key(slot)`, then by slot, smallest first, so results never depend on the order of the candidates. `top_k(k)` is exactly the first `k` results of `within(0)`.
- **Skipped candidates:** those without an embedding, and every candidate when the query has a different dimension.

## Pruning

A scan of a large field is bound by memory bandwidth, so candidates are first ruled out from a quarter of the bytes.

- **Codes.** Each row `r` is kept as int8 codes `k` and a scale `s` with `r̂ = s·k` close to `r`, plus upper bounds on `‖r − r̂‖` and `‖r‖`. The query is quantized the same way.
- **Bound.** `q·r = s_q·s_r·(k_q·k_r) + q̂·(r − r̂) + (q − q̂)·r`. The integer dot product is exact, and Cauchy–Schwarz bounds the other two terms by `‖q̂‖·‖r − r̂‖` and `‖q − q̂‖·‖r‖`. A margin of `n·ε·‖q‖·‖r‖` covers the rounding of the `f32` dot product.
- **Use.** Only a candidate whose bound reaches the threshold reads its `f32` row and gets its exact similarity. The threshold is that of `within`, and for `top_k` the worst similarity kept once `k` are kept. Results are those of the full scan, bit for bit.
- **Tightness.** With unit vectors of 128 dimensions, the bound exceeds the dot product by less than 0.02. On synthetic names at 0.8, fewer than 0.01% of the rows are read in `f32`.

## Cost

A query reads every candidate's codes once, and the `f32` row of those that can reach the threshold: `O(n·d)` operations. All pairs of a set of `n` records take `O(n²·d)`, with the blocked scan of [search](search.md).

The dot product sums eight independent lanes:
- the compiler vectorizes it with the instructions every supported CPU has (SSE2, NEON), with no runtime feature detection;
- it avoids fused multiply-add, which is a slow library call on CPUs without it;
- the summation order is fixed, so results are identical from run to run.

| Measure (Apple M2, one thread) | Result | Budget |
| --- | --- | --- |
| Top-10 over 100,000 vectors of 128 dimensions, median of 25 queries | 0.51 ms; 1.23 ms reading every `f32` row | 5 ms |

## Validation

- `crates/univo-engine/tests/dense_search.rs` covers the search results:
  - `top_k` and `within` against a full sort, with frequent equal similarities, two tie-break keys, candidate subsets, `k` from 0 to beyond the candidates, and several thresholds;
  - independence from candidate order, including equal similarities and equal keys;
  - queries of another dimension;
  - near-duplicates of 128 dimensions, with every similarity of a query used as a threshold, where the bound is tightest.
- `crates/univo-engine/src/quantized.rs` checks the bound against the `f32` dot product, from 1 to 257 dimensions, with near-duplicates, opposite and scaled rows, zero and subnormal vectors, and code sums beyond `i32`. Dropping either Cauchy–Schwarz term makes the search tests fail; dropping the rounding margin does not, because those terms are much larger on the test data.
- The same file covers embedding:
  - normalization-aware deduplication and shared rows;
  - missing texts and texts without known tokens;
  - only semantic fields embedded;
  - batches appended, and batches of another schema, model or dimension refused unchanged;
  - records of another schema, and embedders returning too few rows or rows of another dimension.
- `crates/univo-engine/tests/vector_search_budget.rs` enforces the budget in release builds.
