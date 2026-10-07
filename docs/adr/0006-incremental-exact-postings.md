# 0006. Incremental ordering of exact postings

- Status: Accepted
- Date: 2026-10-07
- Roadmap: U-007, review follow-up

## Context

Re-sorting a dense bucket after every addition made incremental insertion quadratic. On the reference Apple M2, release, one thread, 40,000 identical strings added individually took about 718 ms, versus 1.4 ms in one batch. The benchmark times full `add`, excluding input preparation and index destruction.

## Decision

Append unidentified records to a slot vector. Slots already increase with insertion order. Keep identified postings in a standard-library `BTreeMap<RecordId, usize>`, preserving typed-ID order without a full-bucket sort. Retain global ID uniqueness checks and validation before mutation.

## Alternatives

- Binary insertion into a sorted vector avoids re-sorting but still shifts O(B) entries for decreasing IDs.
- Sorting during search moves work onto the read path and requires additional output selection logic.
- An ordered tree for every record adds allocation and logarithmic work where append order already suffices.

## Evidence

The reproducible release benchmark covers 10k, 20k and 40k records, one batch versus single-record batches, and both no IDs and decreasing integer IDs. A 25 ms budget at 40k leaves room above the measured approximately 2 ms without IDs and 7 ms with IDs. Results verify counts and ordered search after insertion; existing tests retain typed-ID, partition, collision and rollback coverage. See [performance measurements](../performance.md).

## Consequences

Unidentified postings append in amortized O(1); identified postings insert in O(log B). Ordered tree nodes and an additional owned ID key increase identified-index memory. Full equality verification and search output order remain unchanged. No third-party runtime dependency or persistent format is added.

## Revisit when

Measured tree/ID storage materially affects the memory budget, or large batch construction dominates. Any replacement must preserve efficient incremental insertion and deterministic bounded search without moving an unmeasured cost to retrieval.
