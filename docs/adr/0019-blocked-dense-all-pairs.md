# 0019. Blocked dense all-pairs comparisons

- Status: Accepted
- Date: 2026-10-13
- Roadmap: U-003 follow-up, toward the U-020 performance gates

## Context

After [ADR 0018](0018-parallel-all-pairs.md), exhaustive semantic deduplication of 100,000 names took 66.5 s on 8 threads against 14 s for SemHash, and gained only 1.9× from 8 threads. The review of that change noted that "memory bandwidth" was a plausible explanation, not a measured one.

Each all-pairs query scans every later vector of its partition with the scoring cosine (`univo_core::dot`). At 100,000 records of 256 dimensions, that is up to 100 MB per query, for 100,000 queries.

## Decision

- **Measure the cause first.** [`dense_bandwidth`](../../crates/univo-engine/examples/dense_bandwidth.rs) runs the same scan with the vectors in cache or in memory. In memory, the scan stops near the M2's 100 GB/s from 4 threads; in cache it scales to 4.7× ([measurements](../performance.md#blocked-dense-all-pairs-adr-0019)). Memory bandwidth is the bound.
- **Blocks of queries.** `find` and `groups` cut their queries into blocks of 64 consecutive queries of one partition. For each semantic field, a block reads every later vector once and compares it with all its queries, 8 at a time. The per-query candidate lists then replace the dense lane's scan; the other lanes and the scoring are unchanged. A single search keeps the per-query scan.
- **Bounded memory.** A block holds at most 2^20 candidates (4 MiB), over all its queries and semantic fields, plus one row. A block that would hold more is split in two and recomputed, down to one query, which holds what the per-query scan held. Without this bound, a block of similar records holds 64 queries × the partition × the semantic fields before any scoring or pair budget can stop it.
- **Same arithmetic.** The blocked kernel `univo_core::dots` sums the same products in the same eight lanes, in the same order, as `dot`: every similarity is bit for bit the one the per-query scan computes, so the candidates, the scores and the completeness guarantee are unchanged.
- **Explicit vectors.** The kernel uses the four-wide `f32` vectors of the `wide` crate (NEON on arm64, SSE2 on x86-64, plain code elsewhere). Univo's crates still forbid `unsafe`.

## Alternatives

- **Leave the kernel to the compiler.** The same code written on plain arrays ran at 11 ns per pair in one context and 25 to 27 ns in the engine: the vectorizer chose to vectorize across queries, with a shuffle per operation, or kept the sums in memory. Every safe rearrangement tried (lane-outer loops, per-query accumulators, interleaved queries, fixed dimensions) stayed between 13 and 43 ns. Rejected: the speed must not depend on surrounding code.
- **`std::simd`:** not stable in Rust 1.99. Revisit when it is.
- **Intrinsics in Univo (`std::arch`):** the same speed without a dependency, but vector loads need `unsafe` in our crates, which the workspace forbids. Rejected while `wide` gives the same result safely.
- **A matrix-multiplication library (`gemm`, `matrixmultiply`, BLAS):** faster with fused multiply-add, but a different summation order would change scores in their last bits and the guarantee that a score reaching `s` is always found. It would also add a larger dependency. Deferred to quantized vectors, which will need their own exact rescoring anyway.
- **Fused multiply-add in `dot`:** about twice the arithmetic throughput, but it changes every semantic score in its last bits, so it would need a new scoring version. Deferred.

## Evidence

- Micro-benchmark, 100 MB of vectors, 8 threads: 10.1 ns per dot query by query, 2.6 ns in blocks; blocks give the same times in cache and in memory.
- End to end, 100,000 names, 8 threads: 66.5 s → 13.7 s (SemHash 14.0 s), with exact search and the same removed records. 10,000: 0.36 s → 0.23 s; 30,000: 4.1 s → 1.43 s. One thread: 126 s → 57 s.
- Tests:
  - `dots` equals `dot` bit for bit for 1, 4 and 8 queries and lengths 0 to 39 and 255 to 257;
  - `crates/univo-engine/tests/blocked_dense.rs`: `find` equals the exhaustive oracle with two semantic fields and no other lane, partitions of 3 to about 200 records, missing embeddings, shared rows and removed records, outside and inside pools; `groups` equals exhaustive groups.
- Bounded memory: a block over its budget is refused, and split blocks visit every query once, in order, each with all its candidates (unit tests in `blocks.rs`). With eight semantic fields and every pair matching, `find` equals the oracle for budgets of 0, 1,000 and unlimited, and `groups(max_pairs=1)` fails with `limit.group_pairs`.
- Mutation checks, each making the tests fail:
  - skipping the groups of eight or the remaining queries;
  - mixing the halves of a vector in the kernel;
  - ignoring or not counting the block budget;
  - dropping the second half of a split block.

## Consequences

- One new runtime dependency in `univo-core`: `wide`, with `safe_arch` and `bytemuck`. They are pure Rust, under permissive licenses, with no native code ([dependencies](../dependencies.md)).
- Memory per worker: at most 4 MiB of block candidates, or one query's candidates (the partition × the semantic fields) once a block has been split down to one query, as before blocks. Measured with 100,000 identical-vector records, `groups(max_pairs=1)` on 8 threads: without the bound, peak RSS rose by 1,385 MiB above the index with 16 semantic fields (153 MiB with one); with it, by 129 MiB (62 MiB). On the U-003 workload at 100,000 records, peak RSS on 8 threads is 347 MiB, against 314 MiB before blocks.
- Similar data loses part of the blocking: oversized blocks are recomputed in smaller ones. Scoring the many matching pairs dominates there anyway.
- Approximate retrieval (HNSW) keeps its per-query path.

## Revisit when

- Quantized vectors (f16 or int8) land: they change the kernel and need an exact rescoring near the threshold.
- `std::simd` stabilizes: replace `wide`.
- Partitions beyond about 200,000 records: exact all-pairs stays quadratic, so an approximate similarity join is needed there.
