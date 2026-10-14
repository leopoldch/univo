# 0012. Finder-scoped bounded semantic embedding cache

- Status: Accepted
- Date: 2026-10-11
- Roadmap: U-013

## Context

Within-batch encoding already deduplicates text, but later add/search/find calls encode unchanged semantic texts again. The cache must remain local, bounded, model-compatible and independent of Python objects, without introducing a second scoring implementation.

## Decision

Share a lazy Rust `EmbeddingCache` between the indexes of one Finder. Use a verified normalized semantic text key in an immutable full-model namespace. Cache absent vectors too. Refuse incompatible descriptors rather than reusing stale vectors. Current semantic field kind/normalizer version are process-local invariants; no disk state is accepted.

Default to 4 MiB of estimated retained allocation, explicitly configurable or disabled with zero. Account for payload and container capacities, including allocation high-water capacity across hash-table deletion. FIFO eviction keeps hit lookup constant-time. Oversized keys are refused before copying into the cache. Keep inference outside the mutex; duplicate concurrent cold work is allowed and documented. Finder's existing serialization provides sequential reuse.

Scoring/search versions, normalizer behavior, model selection and download budgets are unchanged. Index vectors remain contiguous; the cache does not deduplicate persistent rows across add batches.

## Alternatives

- Unbounded memoization: violates memory constraints.
- Python dict cache: retains Python data, duplicates native ownership and prevents pure Rust reuse.
- Global model/text cache: adds cross-Finder lifetime and locking complexity; scope and ownership are less predictable.
- LRU ordering tree: potentially better under mixed scans/hot keys, but extra per-hit work and metadata. FIFO is sufficient for this measured reuse workload; revisit after real eviction traces.
- Disk cache or snapshot serialization: defer until a separately versioned persistence contract.

## Evidence and validation

[Cache contract](../guide/embedding-cache.md) and [release-wheel measurements](../development/performance.md) describe semantics, accounting and remaining limits. Counting encoders prove no re-encoding of retained normalized texts, including absent-token results; plain/cold/warm vectors are identical. Tests cover growth/eviction, complete model identity, failed encoding and concurrent warm readers. Real-model U-012 results remain identical. No dependency is added.

The budget is a retained estimate rather than a hard RSS/peak bound. Active batches and persistent index copies remain separate memory costs. Failed record insertion can advance cache counters and populate valid entries, while the index stays unchanged.

## Revisit

If measured workloads thrash FIFO, hit-path overhead dominates short inputs, row duplication dominates insertion memory, or U-014/U-015 requires persistent cache identity and mutation-aware lifetime management.
