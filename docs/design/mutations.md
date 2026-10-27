# Mutations by ID

`upsert(records)` replaces the IDs already present and inserts the new ones, and returns the IDs in batch order. `remove(ids)` hides the active IDs at once and returns the missing ones, in input order.
- **Repeated IDs in `remove`.** A present ID repeated in the batch is removed once. Repetitions of a missing ID are all kept in the answer.
- **Typed IDs.** `1` and `"1"` are distinct, and strings are not normalized.

Both methods need `Schema(id_field=...)`. The implicit IDs of string schemas stay reserved to `add`.
- **Batch size.** Batches and generators are bounded to `max_records` items. This capacity bounds **active** records, not resident memory.
- **All or nothing.** A batch that exceeds the budget, holds an invalid ID or fails during encoding publishes no replacement and no removal.
- **Duplicates.** Two identical IDs in one upsert batch are refused: their order does not pick a winner.

```python
from univo import Finder, Schema, Text

finder = Finder(schema=Schema(id_field="id", fields={"name": Text()}))
finder.add([{"id": "a", "name": "Northwind Traders"}])
finder.upsert([{"id": "a", "name": "Contoso"}])
assert finder.remove(["a", "missing"]) == ["missing"]
```

Python's `find(records)` stays temporary: it compares the given batch and does not read the persistent index. Mutations change the data that `search` uses.

## Preparation, generations and search

Conversion, ID and capacity validation, and encoding all happen before publication.
- **Vector reuse.** The Rust engine reuses the vectors of semantic fields whose normalized state is unchanged, even with the cache disabled or after a load. That includes results without known tokens. Other changed fields do not trigger this encoding.
- **Explanations.** They use the new normalization steps, even when the normalized text and the vector are identical.
- **Order of checks.** Model and schema compatibility are checked before existing vectors are copied; capacity is then checked with the reused rows.

**Generations.**
- **When they advance.** The generation advances once per successful non-empty `add` or `upsert`, and per `remove` that removes at least one active ID. An identical upsert counts as a publication. An empty batch, or a removal whose IDs are all missing, does not advance it.
- **Exhaustion.** An exhausted generation is refused before any mutation.
- **Where it appears.** Reports and snapshots carry the current generation.
- **Concurrency.** Searches on one Finder run in parallel; changes run one at a time. The Python runtime is released during Rust work.
- **Rust API.** It requires `&mut SearchIndex` for mutations, and the caller still passes the generation to `save`, as the snapshot contract requires.

**Tombstones.**
- **Replaced and removed versions.** Each one becomes a tombstone. New versions are appended at the end of the store, so the active view follows these publications, while search still sorts by score, then ID.
- **Filtering.** The exact, lexical, value, vector and approximate lanes, the exhaustive oracle and reports all skip inactive slots, including after a change of partition or veto.
- **Budgets.**
  - The exhaustive budget counts active pairs.
  - The semantic report guard counts, per semantic field, the sum over partitions of `active queries × retained slots`. That includes the tombstones scanned, so it stays conservative even though those slots are filtered before any vector computation.
  - The lexical and value limits count the entries actually read, tombstones included: a truncation stays explicit and may lose candidates.

## Retained space and compaction

Before a new `add` or `upsert`, the engine rebuilds its lanes when tombstones reach `max(64, ceil(active / 2))`, or when the batch would exceed twice the capacity in retained slots.
- **No inference.** It rebuilds from the prepared data and the existing vectors.
- **Swap.** The new index is built before it replaces the old one.
- **IDs.** Internal slots may change; external IDs stay stable.
- **Bound.** Publications stay within `2 × max_records` retained slots, under the engine's `u32` limit.

`stats()` exposes the active `records`, `retained_slots` and `tombstones`. `index_bytes` includes the active mask, IDs, postings and retained vectors, but excludes the records and the model.
- **Compaction and the generation.** Compaction changes these physical counters without advancing the logical generation, and may precede an encoding failure.
- **Cache after a failure.** Compatible entries already produced in the cache may also remain after a failure, as for `add`.
- **No partial state.** No partial logical state is ever visible.

The rebuild is amortized but can cause a peak in latency and memory, since the old and new indexes coexist for a while.
- **Removing does not free data at once.** The bounded cache may also keep their texts.
- **No promises.** Univo promises neither secure erasure nor a resident memory ceiling.
- **Open budget.** The 512 MiB memory overhead budget at 100,000 records remains to be demonstrated.

## Snapshots

`save` writes only active records and the vector rows they still reference, remapping rows and streaming the floats.
- **Format.** Format v1 stays compatible: its content is a dense view of the active records, so no tombstone table is needed.
- **Loading.** `load` restores this view with its generation, without inference, and mutations can resume.
- **Durability.** In-memory changes become durable only after a successful `save`. Older generations on disk keep their content, including data removed later in memory: see [persistence](snapshots.md).


