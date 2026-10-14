# Performance measurement

Speed is the main promise of Univo, so resource budgets are tested, not only documented.

## Budgets

[benchmarks/budgets.toml](../benchmarks/budgets.toml) is the single source of enforced budgets. Its values implement the table in [specs/07-performance.md](../specs/07-performance.md). Changing a budget requires updating that specification in the same change, with the reason.

## Reference machine

| Item | Value |
| --- | --- |
| CPU | Apple M2 (4 performance + 4 efficiency cores) |
| Memory | 16 GiB |
| OS | macOS 27.0 |
| Toolchain | Rust 1.99.0, CPython 3.14.7 |
| Power | on mains, no other heavy workload |

Budgets are upper bounds on this machine with release builds.

## Shared CI runners

GitHub-hosted runners are noisy, and some are much slower than the reference machine. CI sets `UNIVO_PERF_SLACK` per platform, which multiplies **time** budgets only: 2.0 in general and 5.0 on macOS x86_64 runners, where preparing the standard workload took up to 393 ms against 85 ms on the reference machine. Every Rust budget test records its measurement in the job summary of the CI run. Size and memory budgets are never relaxed. A failure on a shared runner is confirmed by repetition before it blocks a change; small regressions are detected on the reference machine, not in CI.

## Current measurements

| Measure | Result | Budget | Recorded |
| --- | --- | --- | --- |
| `import univo` p95, 50 fresh interpreters | 0.85 ms | 20 ms | 2026-10-07, U-005 |
| Compressed wheel, macOS arm64 | 0.154 MiB | 30 MiB | 2026-10-07, U-005 |
| `prepare` on the standard workload, 100 000 records, single thread | 845 ns per record (85 ms) | 150 ms | 2026-10-07, U-006 |
| `normalize_text`, 140-byte French description | 150 ns | — | 2026-10-07, U-006 |
| `normalize_text`, ASCII name | 8.7 ns | — | 2026-10-07, U-006 |
| `prepare` on the standard workload, with trigrams for names | 1 100 ns per record (110 ms) | 150 ms | 2026-10-08, U-008 |
| `compare` of standard records of one partition, 1 000 000 pairs, single thread | 311 ns per pair (311 ms) | 550 ms | 2026-10-08, U-008 |
| `compare` of configurations with nested parameters, 1 000 000 pairs, single thread | 143 ns per pair (143 ms) | 250 ms | 2026-10-08, U-008 |

Rust performance budgets live in `crates/*/tests/perf_budget.rs`; they are ignored in debug builds and run by `cargo test --release`. Detailed numbers come from `cargo bench -p univo-core`.

## Dense incremental insertion (U-007 review)

Run `cargo test -p univo-engine --release --locked --test perf_budget -- --nocapture` to compare one batch with single-record batches at 10k, 20k and 40k. Every record has the same normalized text `same`, in one bucket, with either no ID or unique integer IDs inserted in decreasing order. No model, model footprint, network, cache or worker pool is involved. The benchmark times the full `add` path, including hashing, batch validation, allocations and posting insertion; raw-input preparation, index construction/destruction and verification searches are outside the timer. Each sample uses a fresh index. Results are medians of five after one warmup, with sample ranges printed and appended to CI job summaries.

Measured on 2026-10-07: Apple M2, 16 GiB, macOS 27.0.1, Rust 1.99.0, release, one thread. The previous implementation was measured with five samples and no separate warmup; its complete-bucket sort is the regression baseline, not a cross-machine comparison with the reviewer's Linux timings.

| Records | Previous single additions, no IDs | Current one batch, no IDs | Current single additions, no IDs | Current one batch, decreasing IDs | Current single additions, decreasing IDs |
| --- | --- | --- | --- | --- | --- |
| 10,000 | 46.0 ms | 0.350 ms | 0.520 ms | 1.381 ms | 1.591 ms |
| 20,000 | 182.4 ms | 0.521 ms | 0.851 ms | 2.662 ms | 3.055 ms |
| 40,000 | 718.0 ms | 1.146 ms | 1.739 ms | 5.406 ms | 6.199 ms |

At 40k, current single-addition samples ranged from 1.735 to 1.780 ms without IDs and 6.088 to 6.448 ms with decreasing IDs. The 25 ms insertion budget leaves room for allocator/runtime noise while rejecting the previous dense-bucket sorting cost, including the normal CI slack. It is not an end-to-end ingestion or search-latency target. Ordered ID postings add tree-node storage and another owned ID key; total RSS and byte budgeting remain separate work. See [ADR 0006](adr/0006-incremental-exact-postings.md).


## Python Finder (U-011)

[Reproduction script](../benchmarks/performance/python_finder.py), [raw measurements](../benchmarks/performance/u011-macos-arm64.json). On 2026-10-11: Apple M2, 16 GiB RAM, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0; portable CPU release wheel installed outside the repository. Fixed seed 11, 100,000 random 20-character ASCII names and integer IDs, one lexical field, one partition, one thread. Five repeats, 20 warmup queries then 1,000 newly converted queries per repeat (new IDs, one changed character); no model loaded, model footprint zero. Input generation, construction and index destruction are outside add/search timers. The full Python-to-Rust-to-Python path is timed. The final run starts after build/test processes finish; `pmset` reports AC power. Search uses `min_score=0.8`, output limit 10.

| Measurement | Result |
| --- | --- |
| Full `add` of 100,000 records | median 105.1 ms; range 99.6–114.5 ms |
| Full warm `search` p95 | median of five 0.406 ms; range 0.406–0.417 ms |
| Full warm `search` p99 | median of five 0.477 ms; range 0.458–0.489 ms |
| `import univo` p95 | 2.68 ms, 30 fresh interpreters |
| `Finder()` construction p95 | 0.017 ms, 30 fresh interpreters; facade import outside timer |
| Estimated index heap | 26.4 MiB, excluding prepared records and model |
| Process peak RSS | 177.3 MiB, including synthetic Python inputs and all repeat lifetimes |
| Compressed wheel | 1.664 MiB, macOS arm64 |
| Uncompressed wheel / native extension | 3.71 / 3.52 MiB |

All query assertions passed; candidate lanes were untruncated and returned the intended near-match. The random names make candidate retrieval selective: these numbers are a lexical integration benchmark, not the semantic standard workload, dense-match find or a quality estimate. The process RSS includes pre-generated data and the maximum across all repeats; it is not the isolated incremental engine RSS. Other platform wheel sizes and Finder performance have not been measured locally. The full release Rust suite passed before the exact fallback; final affected engine oracle/report tests also pass in release. The final installed wheel passes 1,050 Python/tool/corpus tests (three platform/conditional skips), outside the source tree with Rust absent from PATH. Native architecture checks pass separately.

## Python pair report review (#15)

[Reproduction script](../benchmarks/performance/python_find_report.py), [raw comparison](../benchmarks/performance/u011-report-review-macos-arm64.json). On 2026-10-11, Apple M2 with 16 GiB RAM, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0, portable CPU release wheels installed outside the repository. The baseline is commit `0e3d491`; the correction retains global output in a bounded heap and sorts once. Each of five repeats starts with a new temporary index: 10,000 records, first 150 identical names, then random 20-character ASCII names (seed 15), integer IDs, one lexical field, one partition, one thread, threshold 1, output limit 10,000. No model is loaded (zero model bytes). Input generation and Finder construction are outside timing; conversion, indexing, retrieval and report conversion are included. Build/test processes finished before measurement.

Median full `find` falls from 746.7 ms to 98.8 ms. Both versions count 11,175 pairs, retain 10,000 and report no candidate truncation. Process peak RSS (including Python inputs and repeat lifetimes) is 61.5 MiB before and 62.2 MiB after. This measures an early dense group followed by a sparse tail, not semantic scans, quality or a universal latency guarantee.

The corrected macOS arm64 wheel is 1.663 MiB compressed, with no dependency changes. It passes 1,065 Python/tool/corpus tests outside the source tree with Rust absent from PATH (three conditional skips). The Rust workspace suite and 16 affected engine report/oracle tests in release pass; clippy, mypy, formatting, generated error catalogue and documentation links pass. The semantic guard is also checked across multiple fields and partitions, at zero output and lexical lane limits, and with explicitly raised budgets.

## Semantic embedding reuse (U-013)

[Reproduction script](../benchmarks/performance/python_embedding_cache.py), [raw measurements](../benchmarks/performance/u013-macos-arm64.json). Apple M2 (8 cores), 16 GiB, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0, portable CPU release macOS arm64 wheel installed outside the repository. The pinned default potion-base-8M model is local (29.49 MiB for the three required files), with files already in OS cache. One thread, five repeats per cache setting, fresh Finder per repeat, 1,000 synthetic English descriptions of about 30 words with integer IDs, 1,000 partitions (one target each). Input generation and construction are outside timing. Full add includes local model loading and conversion. Each repeat warms 20 queries, then times 1,000 complete search calls with reused selected text, new query IDs and changed unselected metadata, threshold 0.8, output limit 10.

Median of five search p95 values: 0.0721 ms with cache disabled, 0.0218 ms with the default 4 MiB cache. Median full add including local model load is 59.2 ms without cache and 59.9 ms with cache; this small difference is within measurement noise. The cache retains 1,000 entries in 1,475,218 accounted bytes, with 1,000 misses, 1,020 hits and zero evictions in the final repeat. All queries return the expected record without truncation.

This workload isolates semantic-text reuse rather than large-partition retrieval. It does not imply that cache hits accelerate every 100k search by the same ratio or improve quality. Process peak RSS across both settings/repeat lifetimes is recorded in the raw file; cache bytes are a conservative retained estimate, not an isolated RSS delta or peak bound. The measured compressed wheel is approximately 1.672 MiB; no dependency is added. The real-model U-012 threshold, held-out/group metrics, candidate/oracle results and critical regression measurements remain exactly equal. See the [cache contract](embedding-cache.md).

## Explicit snapshots (U-014)

[Reproduction script](../benchmarks/performance/python_snapshots.py),
[raw measurements](../benchmarks/performance/u014-macos-arm64.json). On
2026-10-12: Apple M2, 16 GiB, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0,
AC power, portable CPU release wheel installed outside the repository. Local
system temporary storage, OS-cache warm files, one thread, five fresh Finder
cycles per size in a fresh process. Synthetic seed 14: integer ID, 100 tenants,
random 20-character lexical name and 32-word semantic description. Default
potion-base-8M at pinned revision bf8b056651a2c21b8d2565580b8569da283cab23,
dimension 256, three model files totaling 29.49 MiB. Input generation is outside
timers. Add includes local model load/conversion/inference; save includes flush,
sync and verification; load includes local model verification/load, decoding,
validation and all secondary-index rebuilding. The original Finder is released
before load. Ten complete reports per cycle remain exactly equal after restore.
No network, cold-disk, power-interruption or universal latency claim.

| Records | Median full add | Median save | Median load | Generation size | Whole-process peak RSS, five cycles |
| --- | --- | --- | --- | --- | --- |
| 1,000 | 60.3 ms | 23.9 ms | 24.1 ms | 1.30 MiB | 110.4 MiB |
| 10,000 | 434.0 ms | 72.6 ms | 74.8 ms | 12.96 MiB | 203.9 MiB |
| 100,000 | 4,066.7 ms | 510.7 ms | 603.7 ms | 129.69 MiB | 960.8 MiB |

The spike first measured 1,087.0 MiB at 100k over five cycles. Bounding temporary
encoding/cache buffers to at most 1,024 texts and approximately 1 MiB of float
output reduces that peak by 126 MiB without changing vectors. The raw file keeps
both measurements. A separate single-cycle 100k process reaches 543.8 MiB after
add and 628.5 MiB after load (629.1 MiB by process end); these are cumulative
high-water marks, not isolated live allocations. Repeated in-process lifetimes
can increase high-water RSS even when the original Finder is released; freeing
objects does not promise that the allocator returns all pages to the OS.

These whole-process figures include Python-generated inputs, the model, records,
lanes, buffers and allocator effects. They do not validate the proposed 512 MiB
**incremental engine** memory target; no isolated RSS-delta gate is claimed and
that target remains unchanged/open. Large partitioned lexical indexes, exact
postings and allocation lifetimes still need profiling before a resource-budget
release claim. `index_bytes` is a retained estimate, not a peak RSS measurement.
The snapshot byte limit similarly does not enforce a process RAM cap.

The macOS arm64 compressed wheel is 1.742 MiB (112 linked packages, no new
transitive/native dependency). Installed-wheel tests pass 1,095 tests on each of
CPython 3.12 and 3.14 outside the source tree with Rust absent from PATH; one
conditional skip each. Rust codec/property tests and affected engine suites pass
in release, including abrupt process exits at twelve boundaries and a real-model
whole-batch versus chunked-row oracle. The [persistence contract](persistence.md)
documents Windows directory-sync and retained-generation limitations. Other
platform results await native CI; no release is published by U-014.

## U-015 mutations and compaction

[Reproduction script](../benchmarks/performance/python_mutations.py) and
[raw samples](../benchmarks/performance/u015-macos-arm64.json). Release abi3 wheel,
Apple M2 / 16 GiB, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0, one thread.
Five fresh processes per size, warm local potion-base-8M files at the U-014
pinned revision (29.49 MiB, dimension 256), cache disabled. Synthetic integer
IDs, 100 partitions, random 20-character lexical names and the U-014 semantic
description workload; generation is outside timers. The final repetitions ran
without concurrent builds/tests. These are workload measurements, not a
universal latency or quality guarantee.

After add, each process upserts one lexical-only batch, changes semantic text
in a separate batch, removes another batch, then performs 60 unchanged upserts.
The latter include periodic full lane/vector compaction without inference.
Ten updated-ID retrieval checks succeed per repetition. Mutation decisions are
also validated separately against a fresh exhaustive index, not inferred from
benchmark speed.

| Initial records | Batch size | Median full add, including model load | Median lexical-only upsert | Median semantic-changed upsert | Median remove | Median unchanged churn | Maximum churn call | Whole-process peak RSS |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1,000 | 100 | 60.6 ms | 0.31 ms | 4.12 ms | 0.014 ms | 0.20 ms | 2.18 ms | 114.8 MiB |
| 10,000 | 1,000 | 425.4 ms | 2.98 ms | 41.41 ms | 0.101 ms | 2.23 ms | 25.19 ms | 312.4 MiB |
| 100,000 | 1,000 | 4,077.0 ms | 3.53 ms | 41.37 ms | 0.152 ms | 3.22 ms | 273.71 ms | 916.0 MiB |

Churn medians are the median of the five within-process medians; maxima and RSS
are the largest observations across repetitions. Each 100k process compacts
once during churn; smaller sizes compact twelve times. Retained slots stay
within twice the active capacity. The 100k result retains 99k active records
and includes the memory spike of rebuilding while the old index is still held.

The RSS figure includes Python inputs, the loaded model, all selected fields,
indexes, matrices and allocator high-water effects. It does **not** demonstrate
the 512 MiB incremental engine target. That budget remains unchanged and open;
compaction latency and peak memory need profiling before release. Removal only
masks data immediately; obsolete records/vectors are reclaimed on a later
mutation's compaction. See the [mutation contract](mutations.md).

Validation: 208 Rust workspace tests pass (nine performance-budget tests ignored
in debug); seven mutation tests and engine snapshot units pass in release.
The four engine performance-budget suites pass in release, and 33 affected
cache/vector/oracle/corpus tests pass in release with the real pinned model.
Installed-wheel suites pass 1,108 tests plus one conditional skip on each of
CPython 3.12 and 3.14, outside the repository with Rust absent from PATH.
Ruff, mypy, fmt, clippy and documentation links pass. The final wheel is 1.762 MiB,
with the same 112 linked packages and no network crate; no dependencies or model
files were added. Cross-platform execution remains native CI's responsibility;
no release or merge is performed by this implementation.

## U-016 explicit HNSW

[Protocol and vector recall](../benchmarks/spikes/u016/README.md),
[script](../benchmarks/performance/python_ann.py) and
[raw samples](../benchmarks/performance/u016-macos-arm64.json). Release abi3
wheel, Apple M2 / 16 GiB RAM, macOS 27.0.1, Python 3.14.7, Rust 1.99.0.
Pinned local potion-base-8M (29.49 MiB, dimension 256), no downloads,
embedding cache disabled. Synthetic ID/tenant/20-character name/32-word
description, fixed seed and threshold 0.9. Query changes one name character
and ID, keeps the semantic description. One warmup and 1,000 complete queries
per process, five fresh processes per configuration. Threads bound encoding;
ANN and scoring stay sequential. These measurements are workload-specific,
not a calibrated business profile or a universal retrieval guarantee.

Each table cell is the median across five processes; brackets give min–max
p95. p99 is published as the median of per-process p99 and is exploratory
with 1,000 samples. Timed add includes model loading and graph construction.
RSS is handled separately because snapshot reconstruction is optional.
Local test/build activity occurred during parts of graph construction;
the full release suite ran between processes with the benchmark paused.
No query benchmark ran concurrently with that release suite. The temporary-find
admission guard was tightened during this measurement campaign; it is not
used by the persistent add/search workload reported here.

| Records | Threads | Retrieval | Add median (ms) | Search p95 median [min–max] (ms) | Search p99 median (ms) |
| --- | --- | --- | --- | --- | --- |
| 1,000 | 1 | exact | 64.12 | 0.110 [0.107–0.114] | 0.128 |
| 1,000 | 1 | hnsw | 64.23 | 0.108 [0.103–0.110] | 0.125 |
| 1,000 | 4 | exact | 42.30 | 0.117 [0.116–0.118] | 0.131 |
| 1,000 | 4 | hnsw | 42.76 | 0.118 [0.117–0.120] | 0.131 |
| 10,000 | 1 | exact | 450.53 | 0.358 [0.356–0.362] | 0.394 |
| 10,000 | 1 | hnsw | 5,722.32 | 0.576 [0.544–0.710] | 0.789 |
| 10,000 | 4 | exact | 234.84 | 0.373 [0.361–0.377] | 0.432 |
| 10,000 | 4 | hnsw | 5,581.95 | 0.579 [0.555–0.627] | 0.790 |
| 100,000 | 1 | exact | 4,265.24 | 3.146 [3.100–3.573] | 3.594 |
| 100,000 | 1 | hnsw | 118,149.96 | 2.600 [2.410–3.112] | 3.112 |
| 100,000 | 4 | exact | 2,069.15 | 3.133 [3.078–3.140] | 3.582 |
| 100,000 | 4 | hnsw | 116,082.46 | 2.626 [2.516–2.781] | 3.128 |

At 1k no graph is allocated. At 10k the dense scan is faster; 10k is
only the small-partition safeguard, not a proven optimal crossover.
The dense default remains unchanged and `auto` is deferred.

The standard 100-tenant workload at 100k has only 1k records per partition,
so even an explicitly requested HNSW profile allocates no graph:

| Threads | Retrieval | Add median (ms) | Search p95 median [min–max] (ms) | Search p99 median (ms) |
| --- | --- | --- | --- | --- |
| 1 | exact | 4,405.58 | 0.275 [0.266–0.292] | 0.419 |
| 1 | hnsw | 4,416.03 | 0.282 [0.270–0.295] | 0.459 |
| 4 | exact | 2,185.62 | 0.285 [0.274–0.307] | 0.413 |
| 4 | hnsw | 2,186.30 | 0.285 [0.281–0.308] | 0.474 |

All 80,000 queries retain their known positive and return no other records
on this fixture; all 800 exhaustive audit queries agree. The lexical lane
can independently recover the positive, so this result does not prove
vector candidate recall or repair the failed U-012 quality gates.
Vector recall is separately measured on held-out ROR names: strict
recall@10 99.9951% at ef512, five processes with 2,048 queries each.
That background contains only 8,707 vectors: neither that test nor this
one-positive hybrid fixture proves 99% vector recall at 100k.

Whole-process maximum RSS at 100k, **one partition**, grouped by phase scope:

| Threads | Retrieval | Without snapshot (MiB, repetitions) | With snapshot/load (MiB, repetitions) |
| --- | --- | --- | --- |
| 1 | exact | — | 670.4, 5 |
| 1 | hnsw | 573.4, 3 | 887.5, 2 |
| 4 | exact | 412.1, 5 | — |
| 4 | hnsw | 576.3, 5 | — |

RSS includes Python inputs, selected records, model, vectors, lanes, graph
copies and allocator high-water effects. With load, both old and restored
Finder coexist. These scopes are not interchangeable; none is an isolated
incremental engine RSS gate. The 512 MiB target remains unchanged/open.
Graph retained-byte statistics are rough estimates, not a memory ceiling.

HNSW save/load were sampled 2 times at 100k, one thread: median save 539.22 ms, load including graph reconstruction 122.01 s. This is a phase-cost observation, not a five-repetition p95. Smaller snapshots falling below 10k restore the dense fallback; approximate retrieval can change without a generation change.

At 100k one-thread HNSW, 100 unchanged-vector upserts take median 0.680 ms and 100 removals 0.031 ms. Compaction still reconstructs the full graph and can be expensive; no mutation-latency guarantee follows.

Validation: 219 Rust tests in debug; the full release workspace passes
227 tests including all nine performance budgets and real-model tests.
After adding preliminary admission, all six ANN integration tests pass
in release. Installed-wheel suites pass 1,121 Python tests plus one
conditional skip on each of CPython 3.12/3.14, installed outside the source
tree with Rust absent from PATH. Clippy, fmt, Ruff, mypy, links and native
network audit pass. Local macOS arm64 wheel 1.789 MiB versus 1.762 MiB
U-015; 115 linked crate names versus 112. GitHub Actions jobs did not start
because account payments/spending limits block execution: no cross-platform
CI success is claimed. No model change, release or merge.

## Parallel all-pairs reports (ADR 0018)

[Script](../benchmarks/comparison/scale.py), [raw results](../benchmarks/performance/all-pairs-macos-arm64.jsonl), U-003 protocol: synthetic company names in one partition, one record in ten a near-duplicate with a typo, `min_score=0.8`, pairs turned into removals by SemHash's rule inside the timed section. Apple M2 (4 performance and 4 efficiency cores), 16 GiB, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0, release build, potion-base-8M at `bf8b0566` in OS cache, one run per line, one process at a time. Wall time includes model loading; CPU time covers deduplication on every thread.

| Records | Configuration | Before (U-003) | 1 thread | 8 threads | CPU, 8 threads | Peak RSS, 8 threads |
| --- | --- | --- | --- | --- | --- | --- |
| 10,000 | lexical | 2.3 s | 1.1 s | **0.23 s** | 1.5 s | 43 MiB |
| 10,000 | semantic, exact | 2.6 s | 1.4 s | **0.36 s** | 2.1 s | 132 MiB |
| 10,000 | semantic, HNSW | 6.4 s | | 4.2 s | 8.4 s | 149 MiB |
| 30,000 | lexical | 18.8 s | 9.7 s | **1.9 s** | 12.7 s | 76 MiB |
| 30,000 | semantic, exact | 23.0 s | 11.7 s | **4.1 s** | 27.7 s | 179 MiB |
| 30,000 | semantic, HNSW | 39.7 s | | 20.4 s | 58.7 s | 231 MiB |
| 100,000 | lexical | 281 s | 124 s | **24.4 s** | 174 s | 182 MiB |
| 100,000 | semantic, exact | 251 s | 126 s | **66.5 s** | 485 s | 314 MiB |
| 100,000 | semantic, HNSW | 220 s | | 113 s | 343 s | 518 MiB |

Every configuration removes the same records as before. For reference, SemHash 0.5.0 takes 1.0 s, 2.5 s and 14 s on the same data with every core ([U-003](../benchmarks/comparison/README.md)).

- **Half the comparisons.** An all-pairs query now scans only the records after it, in the lexical verification and the dense semantic scan: one thread is about twice as fast.
- **Threads.** Lexical reports scale to 5.1× on 8 threads at 100,000 records. Dense semantic reports stop at 1.9× there: each query streams every later vector from memory, up to 100 MB at 100,000 records, so memory bandwidth, not arithmetic, bounds them from about 4 threads. [ADR 0019](adr/0019-blocked-dense-all-pairs.md) measures this cause in isolation and compares blocks of queries with each vector read.
- **HNSW** stays slower than the exhaustive scan up to 30,000 records; its graph is still built on one thread.
- **Small calls.** With `threads=1`, a lexical index creates no pool: `find` of 20 records takes 483 µs against 510 µs before, and `group` 83 µs against 85 µs (median of five runs of 2,000 calls). With `threads=4`, creating the index's pool adds about 25 µs to `find` and 50 µs to `group`.

## Blocked dense all-pairs (ADR 0019)

**Cause, in isolation.** [`dense_bandwidth`](../crates/univo-engine/examples/dense_bandwidth.rs) times the scoring cosine of 256-dimensional vectors, query by query or in blocks of 64 queries sharing each row read, with the vectors in cache (1 MB) or in memory (100 MB, a 100,000-record partition). [Raw output](../benchmarks/performance/dense-bandwidth-macos-arm64.csv); best of three, Apple M2 on battery at full charge, low-power mode off.

| Vectors | Mode | 1 thread | 4 threads | 8 threads | Read on 8 threads |
| --- | --- | --- | --- | --- | --- |
| 1 MB | query by query | 19.2 ns/dot | 5.7 ns | 4.1 ns | 253 GB/s (cache) |
| 100 MB | query by query | 20.5 ns | 8.3 ns | 10.1 ns | 102 GB/s |
| 1 MB | blocks of 64 | 10.9 ns | 3.3 ns | 2.4 ns | 6.6 GB/s |
| 100 MB | blocks of 64 | 11.3 ns | 3.3 ns | 2.6 ns | 6.1 GB/s |

Query by query, the in-memory scan stops near the M2's nominal 100 GB/s from 4 threads, and 8 threads are slower than 4; in cache the same work scales to 4.7×. In blocks, cache and memory give the same times: the scan no longer depends on memory bandwidth.

**End to end.** U-003 workload (synthetic names, one partition, `min_score=0.8`, [script](../benchmarks/comparison/scale.py)), model load included, same conditions; [raw results](../benchmarks/performance/blocked-dense-macos-arm64.jsonl).

| Records | Before, 1 thread | Blocked, 1 thread | Before, 8 threads | Blocked, 8 threads | CPU, 8 threads | Peak RSS, 8 threads | SemHash 0.5.0 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 10,000 | 1.4 s | 0.76 s | 0.36 s | **0.23 s** | 1.2 s | 131 MiB | 1.0 s |
| 30,000 | 11.7 s | 5.4 s | 4.1 s | **1.43 s** | 9.2 s | 179 MiB | 2.5 s |
| 100,000 | 126 s | 57 s | 66.5 s | **13.7 s** | 100 s | 347 MiB | 14.0 s, 643 MiB |

Every run removes the same records as before. Univo's search is exact; SemHash's index is approximate.

**Memory of similar data.** Blocks hold at most 2^20 candidates and split when they would hold more. 100,000 records with identical vectors, `groups(max_pairs=1)` on 8 threads, peak RSS above the index: 1,385 MiB with 16 semantic fields before the bound, 129 MiB with it; 153 MiB and 62 MiB with one field.

The all-pairs harness [`all_pairs`](../crates/univo-engine/examples/all_pairs.rs) gives 11.2 ns per pair on one thread at 20,000 records, against 26.5 ns for the same kernel left to the compiler's vectorizer.

## Multilingual default model (ADR 0021)

The default model now has 128 dimensions instead of 256 ([ADR 0021](adr/0021-multilingual-default-model.md)). Model-level budgets, release build, same reference machine:

| Measure | potion-base-8M | New default |
| --- | --- | --- |
| Load, verify and encode one text, median of 5 | 18 ms | 60 to 95 ms |
| 20,000 descriptions, 1 thread / 8 threads | 0.95 s / — | 0.82 s / 0.27 s |
| Exhaustive top-10 over 100,000 vectors, one thread | 2.5 ms | 1.2 ms |
| Search p95 at 100,000 records, one partition / ten / a group of 10,000 | 10.0 / 2.3 / 8.5 ms | 8.5 / 1.65 / 6.6 ms |
| Indexes for that workload | 156 MiB | 92 MiB |

Loading is slower because the tokenizer and the vocabulary are larger (105,879 tokens); it happens once per Finder, at the first semantic use.

**End to end.** Same U-003 workload as above, every core, model load included. Both tools use the new default model. Release wheel installed outside the tree, SemHash 0.5.0, on battery at 96%, low-power mode off; [raw results](../benchmarks/performance/default-model-macos-arm64.jsonl).

| Records | Univo, potion-base-8M | Univo, new default | CPU | Peak RSS | SemHash, new default |
| --- | --- | --- | --- | --- | --- |
| 10,000 | 0.23 s | **0.23 s** | 0.7 s | 154 MiB | 1.24 s, 157 MiB |
| 30,000 | 1.43 s | **1.02 s** | 5.0 s | 184 MiB | 2.21 s, 226 MiB |
| 100,000 | 13.7 s | **8.17 s** | 50.9 s | 301 MiB | 8.72 s, 491 MiB |

Deduplication CPU time halves with the dimension. Univo's search stays exact and SemHash's approximate. Their thresholds differ, so the removal counts differ (9,688 and 4,120 at 100,000 records): this compares cost, not quality.

## Acronym evidence (ADR 0022)

Release builds, same reference machine, median of five runs, alternating `main` and the change three times.

| Measure | `main` | With acronym evidence |
| --- | --- | --- |
| Prepare 100,000 standard records | 1,140 ns per record | 1,290 ns per record |
| Compare one million standard record pairs | 327 ns per pair | 333 ns per pair |
| Compare one million configuration pairs | 157 ns per pair | 158 ns per pair |
| Lexical all-pairs at 100,000 records, every core | 26.4 s | 26.1 s |
| Semantic all-pairs at 100,000 records, every core | 8.17 s | 8.15 s |

- **What the cost buys.** The acronym and initials of each text are hashed once, at preparation, so that a comparison usually costs a few integer comparisons.
- **Preparation.** A first version that built them as strings doubled preparation (2,500 ns per record) and added 36 ns per compared pair.
- **The end-to-end runs.** They ran on battery and removed the same records. Their synthetic names contain no acronyms, so the acronym lane stays empty.
