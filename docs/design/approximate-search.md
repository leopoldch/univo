# Approximate semantic search

Approximate retrieval is an explicit profile; the default remains `retrieval="exact"`.
- **What "exact" means.** It refers to complete vector retrieval, not a guarantee that two objects designate the same entity.
- **The oracle.** `exhaustive=True` scores every admissible record, independently of the lanes.

```python
finder = Finder(
    schema=schema,
    retrieval="hnsw",
    ann_ef_search=512,
    ann_max_candidates=1024,
    offline=True,
)
```

**When graphs are used.**
- **Large partitions.** `Text(mode="semantic")` fields use one HNSW graph per field and typed partition, from 10,000 retained slots in the partition.
- **Smaller partitions** use the complete vector scan.
- **What the threshold is for.** It avoids graphs for small tenants. It is neither an `auto` choice nor a promise of a gain at 10,000 records: the measurements show workloads where the complete scan stays faster.
- **Hybrid fields** stay lexical.
- **Partitions** are checked by value before traversal, even when fingerprints collide.
- **No allocation without need.** Without embeddings or graphs, no approximate index is allocated, and import and construction load no model.

**Parameters.** `ann_ef_search` is an integer from 1 to 8,192, and `ann_max_candidates` from 1 to 100,000. Both are validated before the model is resolved.
- **Expansion.** The backend may raise `ef` to the number of neighbours requested. The search first requests at most 32 distinct vectors, then doubles while the last neighbour passes the semantic threshold, up to the configured ceiling.
- **Identical vectors** share a node but keep their distinct slots.
- **The ceiling** applies to the records of each approximate lane, after inactive versions and the query's ID are excluded.
- **Other lanes.** The exact, lexical and value lanes always join the union, so the total number of candidates may exceed that ceiling.
- **Scoring.** Scoring and vetoes stay those of the Rust core. A veto may reduce the results of a top-k; no completeness under constraints is promised.

**Reporting the approximation.**
- **Profile.** As soon as a graph is used, `Report.search_profile == "approximate"`, and a warning reports possible omissions **even when `candidate_truncated` is false**.
- **`candidate_truncated`** reports an observed saturation, distinct from the inherent approximation and from `output_truncated`.
- **`total_matches`** counts only the matches actually found.
- **`exhaustive=True`** ignores the graphs and keeps its explicit limits.
- **Observable profile.** The generation, the configuration fingerprint, the parameters and the backend's versioned identity make the profile observable.

**The `Finder(max_semantic_comparisons=...)` budget remains mandatory.** It applies to `find` and `group`.
- **Two checks.** A first check by partition refuses the dense minimum (queries × slots × semantic fields) before encoding and graph construction. A final check adds the levels and retries observed after construction.
- **What it bounds.** This budget does not cap the cost of resolving, preparing or building the indexes.
- **Exhaustive runs.** A temporary exhaustive `find` builds no graph.
- **How it counts.** It counts active queries × the entries of the complete scan, or an HNSW bound covering every node, level, cosine check and adaptive retry. Taking the maximum with the retained slots preserves the dense budget.
- **`limit`** does not bound the computation. Raising the budget explicitly allows large reports; the approximate index does not bypass this protection.

## Mutations and snapshots

`add` and `upsert` insert the new vectors into the graphs concerned, and `remove` hides old slots in every lane.
- **Stale nodes.** They may use up neighbours and reduce recall until compaction, but can never return a removed record.
- **Compaction.** It rebuilds the graphs from the active view without inference; old and new indexes coexist for a while.

Snapshots keep the vectors and an optional `ann` identity in the v1 manifest: backend and kernel, metric, seed, construction parameters, partition threshold and search options.
- **No graph on disk.** They do not serialize the dependency's graph. Loading validates this identity, then rebuilds the graphs from the vectors without re-encoding the records.
- **Incompatibility** is a typed error.
- **Older readers.** Snapshots of the default dense profile keep their manifest without this property and stay readable by earlier readers. An earlier reader refuses an approximate snapshot it does not know.

A rebuild after mutations or a restore may change the approximate recall, the topology and the use of the small-partition fallback, without changing the logical generation. Exhaustive results stay stable. The graphs use a fixed seed, but approximate results are not promised to be bit-identical across platforms or physical layouts.

## Cost

**Backend.** `fast-hnsw` 1.0.1, M = 16, efConstruction = 200, `Heuristic` pruning, seed 16. The distance is the true Euclidean distance, accumulated portably over eight lanes.
- **Final score.** It uses the core's cosine. Validated vectors are unit vectors.
- **Requirements.** No particular CPU instruction, no unsafe Univo code, no extra worker pool and no persistence of the dependency is needed.
- **Threads.** One search is sequential. `threads` bounds inference and the parallel queries of `find` and `group`. Graph construction stays sequential.

**Memory.** The graph copies the distinct vectors and stores links, hashes and slots.
- **`stats()["ann_nodes"]`** counts distinct vectors, tombstones included.
- **`stats()["ann_bytes"]`** is a rough estimate of the graphs' retained memory, included in `stats()["index_bytes"]`.
- **Not a memory ceiling.** It excludes the model, the selected records, Python inputs, the temporary allocation of visited sets and rebuilds.
- **Open budget.** The 512 MiB total incremental budget remains to be demonstrated.

Wheel and full resident-memory measurements are published separately.
