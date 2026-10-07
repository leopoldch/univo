# 0005. Verified exact-index keys and record identity

- Status: Accepted
- Date: 2026-10-07
- Roadmap: U-007

## Context

U-006 supplies normalized records and pairwise exact equality. U-007 needs candidate retrieval while preserving typed values, missing states, partitions and normalization evidence. Python orchestration and persistent mutations belong to later tasks.

## Decision

Place the index in `univo-engine`, using only `univo-core`. Reuse the existing typed, length-delimited hashing helpers for an in-memory key covering partition and selected fields. Exclude IDs and explanation traces. Verify every candidate with `exact_match`; the key never establishes equality.

Keep external IDs typed and unique across the instance. Validate finite batches before insertion. Records without IDs use temporary slots; persistent generated IDs remain part of U-011. Bound output explicitly while scanning every candidate and reporting omitted hits. Use a record-count capacity without claiming a total memory budget.

## Alternatives

- Pairwise scans need no index but repeat comparisons across unrelated content.
- Storing full canonical keys duplicates selected values; fingerprints retain slot postings while records remain the source of truth.
- Hash-only equality saves verification but permits false matches under collisions.
- Implementing the index inside the Python binding would duplicate logic and prevent reuse by other bindings.

## Evidence

Differential tests compare indexed retrieval with the pairwise oracle on the critical corpus and generated typed records. A test puts unequal records and different partitions into one bucket deliberately. Regression cases cover duplicate IDs, failed batches, missing states, signed zero, normalization explanations and truncated output. These are correctness checks, not a retrieval-performance benchmark or model-selection result.

## Consequences

No new runtime dependency or persistent format is introduced. Search results are deterministic. Fingerprints do not have a stable on-disk contract and dense/colliding buckets still require full verification. Byte budgets, durable IDs, snapshots and concurrent mutation remain separate work.

## Revisit when

Profiling shows hashing or dense bucket sorting dominates, hostile collision workloads require stronger hashing, or U-011/U-014 introduce memory budgeting and persistent identity. Preserve collision verification and exact semantics in every replacement.
