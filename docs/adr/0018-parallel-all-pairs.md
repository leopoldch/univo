# 0018. Parallel all-pairs reports

- Status: Accepted
- Date: 2026-10-12
- Roadmap: U-003 follow-up, toward the U-020 performance gates

## Context

The [U-003 comparison](../../benchmarks/comparison/README.md) measured SemHash 9 to 16 times faster than Univo on all-pairs deduplication of one large partition. Two causes were in Univo itself:
- `find` and `group` ran every query on one thread, while SemHash used 5.5 to 6.2 cores. `threads` only bounded semantic encoding.
- Each all-pairs query scanned its whole partition, although a pair is only kept from its lower slot: every lexical verification and dense cosine was computed twice.

[specs/05-rust-architecture.md](../../specs/05-rust-architecture.md) asks that `threads` control the global budget, without multiplying pools, and AGENTS.md forbids nested oversubscription.

## Decision

- **Parallel queries.** `SearchIndex::find` and `SearchIndex::groups` run one query per stored record (per exact group for `groups`) in parallel when called from a Rayon pool worker, and in order on the calling thread otherwise. The engine never uses Rayon's global pool implicitly: callers choose the threads by installing a pool.
- **Thread budget.** In Python, `threads` now bounds the all-pairs reports as well as encoding. A single search (`search`, `check_or_add`) still runs on the calling thread; concurrent calls parallelize it ([ADR 0016](0016-local-concurrency-and-check-or-add.md)). An index without semantic fields and `threads=1` creates no pool. Semantic indexes keep one, as before, because the tokenizer would otherwise use every core.
- **Same result on any number of threads.**
  - `find` keeps the best `limit` pairs per worker and merges them under the existing total order (score, left slot, right ID, right slot).
  - `groups` folds qualifying pairs into one shared map, which only grows. Workers buffer at most 1,024 pairs before taking its lock, and the map checks `max_pairs` on each insertion, so `limit.group_pairs` fails exactly when more than `max_pairs` distinct pairs qualify.
- **Triangular scan.** An all-pairs query verifies lexical candidates and computes dense cosines only for records after it. Lexical entries before it are still read and counted against `max_entries`, so truncation is unchanged. Partition slot lists are in increasing order, so the dense scan starts at a binary-searched position.

## Alternatives

- **Rayon's global pool:** simpler, but it would use every core whatever `threads` says, and nest with callers' pools. Rejected.
- **One edge map per worker, merged at the end:** no lock, but each worker could hold up to `max_pairs` pairs, so the budget would bound memory only per thread, and the error would depend on the schedule. Rejected.
- **One pool shared by all indexes of a Finder or process:** removes the 25 to 50 µs of pool creation on small calls with `threads > 1`, but concurrent `find` calls would then wait for each other's workers. Deferred until measured on real workloads.
- **Changing the default `threads`:** SemHash uses every core by default. The default stays 1 here; a new default is an API decision, made separately.

## Evidence

Measured on the U-003 workload, Apple M2, release build ([performance](../performance.md#parallel-all-pairs-reports-adr-0018)):

| 100,000 records, one partition | Before | 1 thread | 8 threads |
| --- | --- | --- | --- |
| lexical | 281 s | 124 s | 24.4 s |
| semantic, exact | 251 s | 126 s | 66.5 s |

At 10,000 records, 8 threads take 0.23 s (lexical) and 0.36 s (semantic, exact), against 1.0 s for SemHash. Every run removes the same records as before.

Tests (`crates/univo-engine/tests/parallel_all_pairs.rs`, `tests/python/test_concurrency.py`):
- `find` and `groups` give identical results on 1, 2, 4 and 8 threads and outside any pool, and `find` equals the exhaustive oracle;
- each lane alone finds pairs in adjacent slots, where a query's scan starts;
- the group pair budget fails at exactly the number of qualifying pairs, with more than two worker buffers in flight.

Mutation checks:
- dropping one worker's heap at merge fails the tests;
- starting the dense or lexical scan one slot too late fails them;
- loosening the group budget by one fails them.

## Consequences

- `find` holds up to `threads × limit` pairs with explanations before the merge, instead of `limit`. `groups` holds `max_pairs` pairs plus 1,024 per worker.
- Dense semantic reports are bound by memory bandwidth from about 4 threads: each query streams every later vector. Scaling is 1.9× on 8 threads at 100,000 records, against 5.1× for lexical reports.
- HNSW graph construction stays on one thread.

## Revisit when

- Blocked dense comparisons (several queries per vector read) land: re-measure thread scaling.
- Small `find` or `group` calls with `threads > 1` dominate a workload: share pools.
