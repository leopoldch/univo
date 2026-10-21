# Embedding cache

Each Finder has a lazy, Rust-only cache shared by its persistent index and temporary `find` indexes. Only semantic fields use it. The default retained budget is 4 MiB; disable retention with `Finder(embedding_cache_bytes=0)`. Construction and import create no cache storage, load no model and access no network.

```python
from univo import Finder, Schema, Text

finder = Finder(
    schema=Schema(fields={"description": Text(mode="semantic")}),
    embedding_cache_bytes=4 << 20,
    offline=True,
    model_path="/path/to/preloaded/model",
)
finder.add([{"description": "local search"}])
finder.search({"description": "LOCAL SEARCH", "ignored": "changed"})
print(finder.stats()["embedding_cache_hits"])
```

## Identity and invalidation

The key is the normalized semantic text, verified by full string equality even on hash collisions. The namespace is the complete immutable model descriptor: source/revision, all file digests (including tokenizer and encoding configuration), dimension and backend version. Only semantic text prepared by the current Rust normalizer enters this process-local cache; field kind and normalizer version are implicit in that namespace. There is no disk cache or cross-version state to reuse.

Field paths, weights, IDs, partition keys and unselected fields do not change the encoder's output for the same normalized text. Identical semantic texts can therefore share inference across fields/partitions while matching still enforces all partitions and vetoes. A changed selected text misses; unchanged selected texts hit. Missing/blank fields are not encoded, and texts with no known model token cache an absent vector, preserving missing-evidence semantics.

A cache from another model, tokenizer, configuration, dimension or backend is refused with `state.model_mismatch`, before a hit can be consumed. Create a new cache/index when changing that namespace. Python Finder's model and schema remain fixed; record upsert/remove are covered by the [mutation contract](mutations.md).

## Eviction and memory

Eviction is FIFO by successful insertion; hits do not refresh the queue. This keeps hit lookup constant-time without another ordering tree or dependency. Re-encoding after eviction is expected. Admission first checks whether an entry fits an empty cache, including its containers. Their minimum allocation estimate is measured once from standard-library capacities. An impossible entry leaves existing entries and eviction counters untouched; its text key is not copied into the cache.

`embedding_cache_bytes` bounds a conservative **retained allocation estimate**: text bytes, vector bytes, 64 bytes per entry for Arc headers/alignment, hash-table capacity/control-byte allowance and FIFO backing capacity. Hash-table allocation high-water capacity is tracked because deletion can lower reported capacity without freeing storage. Admission evicts until the estimate fits; if necessary empty containers are released. The estimate excludes fixed cache/model-descriptor metadata, persistent index vectors, active batch/query buffers and allocator-specific overhead. It is not a process RSS or peak-allocation cap; container resizing and in-flight rows can temporarily exceed the retained estimate.

The cache holds no Python objects. Rows use immutable Rust Arcs during lookup/publication and are copied into contiguous index buffers for dense search. Cached entries are independent of index lifetimes; this milestone does not add deduplication of index rows across separate add batches.

## Diagnostics and concurrency

`Finder.stats()` adds `embedding_cache_hits`, `embedding_cache_misses`, `embedding_cache_evictions`, `embedding_cache_entries` and `embedding_cache_bytes`. Hits/misses count distinct normalized, nonblank texts per field request, after existing within-batch deduplication. With cache disabled or no semantic field, all counters remain zero. `index_bytes` excludes cache storage; its estimate is exposed separately. Cache capacity appears in `Finder.config()` but does not change the scoring/configuration fingerprint.

Cache lookup/publication uses a Rust mutex; inference runs outside it. Concurrent cold requests from Rust can encode the same missing text, while warm readers reuse it safely. Python Finder already serializes operations, so sequential calls reuse every retained text without inference. A poisoned mutex returns a typed error, never a panic across FFI.

Atomic add and read-only search describe the record index, not the memoization counters: cache diagnostics can advance and successfully encoded texts can remain cached even if later record insertion fails. Scoring, ranking, truncation and generation behavior are unchanged. The cache is an optimization, not another source of matching evidence.

## Verification

Counting Rust encoders prove reuse across add/query/indexes, normalized Unicode equality, absent vectors, FIFO re-encoding and synchronized warm reads. Budget tests exercise tiny limits, container growth and oversized keys; compatibility tests change file digests, dimensions, backend and source. Plain, cold and warm vectors agree exactly. Installed-wheel tests cover field-local changes, unselected data, disabled retention, limits, diagnostics and normalization explanations.

The real default-model quality evaluator reproduces the published quality threshold, held-out and group metrics, candidate/oracle counts and critical regression results exactly. [Performance measurements](../development/performance.md) isolate repeated semantic text reuse; they do not imply a speedup for every large-partition search.

Temporary field encoding is bounded to at most 1,024 distinct texts per call and approximately 1 MiB of float output (one row minimum), instead of retaining a corpus-sized encoder matrix and Arc row list. Deduplication/row references remain global and a late failure leaves the index unchanged; earlier cache publication may remain. This controls active inference buffers, not the whole index or RSS. See the [snapshot measurements](../development/performance.md).

`upsert` also reuses stored vectors for unchanged normalized semantic fields, independently of cache contents or eviction. Removed records disappear from search immediately, but compatible text entries may remain in the bounded cache until eviction; no secure-erasure guarantee is made. See [mutations](mutations.md).
