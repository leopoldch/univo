# Performance measurement

Speed is the main promise of Univo, so resource budgets are tested, not only documented.

## Budgets

[benchmarks/budgets.toml](../../benchmarks/budgets.toml) is the single source of enforced budgets. Its values implement the table in [specs/07-performance.md](../../specs/07-performance.md). Changing a budget requires updating that specification in the same change, with the reason.

Timed tests hold `univo_testkit::timing::exclusive()` while they measure, so the timed tests of one test binary never share the CPU. Without it, Cargo ran them in parallel, and preparing the standard workload measured 1,295 ns per record instead of 1,175 ns, up to 1,950 ns on a busy machine, against a 1,500 ns budget.

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

At 40k, current single-addition samples ranged from 1.735 to 1.780 ms without IDs and 6.088 to 6.448 ms with decreasing IDs. The 25 ms insertion budget leaves room for allocator/runtime noise while rejecting the previous dense-bucket sorting cost, including the normal CI slack. It is not an end-to-end ingestion or search-latency target. Ordered ID postings add tree-node storage and another owned ID key; total RSS and byte budgeting remain separate work. See [ADR 0006](../adr/0006-incremental-exact-postings.md).


## Python Finder (U-011)

[Reproduction script](../../benchmarks/performance/python_finder.py), [raw measurements](../../benchmarks/performance/finder-macos-arm64.json). On 2026-10-11: Apple M2, 16 GiB RAM, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0; portable CPU release wheel installed outside the repository. Fixed seed 11, 100,000 random 20-character ASCII names and integer IDs, one lexical field, one partition, one thread. Five repeats, 20 warmup queries then 1,000 newly converted queries per repeat (new IDs, one changed character); no model loaded, model footprint zero. Input generation, construction and index destruction are outside add/search timers. The full Python-to-Rust-to-Python path is timed. The final run starts after build/test processes finish; `pmset` reports AC power. Search uses `min_score=0.8`, output limit 10.

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

[Reproduction script](../../benchmarks/performance/python_find_report.py), [raw comparison](../../benchmarks/performance/find-report-macos-arm64.json). On 2026-10-11, Apple M2 with 16 GiB RAM, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0, portable CPU release wheels installed outside the repository. The baseline is commit `0e3d491`; the correction retains global output in a bounded heap and sorts once. Each of five repeats starts with a new temporary index: 10,000 records, first 150 identical names, then random 20-character ASCII names (seed 15), integer IDs, one lexical field, one partition, one thread, threshold 1, output limit 10,000. No model is loaded (zero model bytes). Input generation and Finder construction are outside timing; conversion, indexing, retrieval and report conversion are included. Build/test processes finished before measurement.

Median full `find` falls from 746.7 ms to 98.8 ms. Both versions count 11,175 pairs, retain 10,000 and report no candidate truncation. Process peak RSS (including Python inputs and repeat lifetimes) is 61.5 MiB before and 62.2 MiB after. This measures an early dense group followed by a sparse tail, not semantic scans, quality or a universal latency guarantee.

The corrected macOS arm64 wheel is 1.663 MiB compressed, with no dependency changes. It passes 1,065 Python/tool/corpus tests outside the source tree with Rust absent from PATH (three conditional skips). The Rust workspace suite and 16 affected engine report/oracle tests in release pass; clippy, mypy, formatting, generated error catalogue and documentation links pass. The semantic guard is also checked across multiple fields and partitions, at zero output and lexical lane limits, and with explicitly raised budgets.

## Semantic embedding reuse (U-013)

[Reproduction script](../../benchmarks/performance/python_embedding_cache.py), [raw measurements](../../benchmarks/performance/embedding-cache-macos-arm64.json). Apple M2 (8 cores), 16 GiB, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0, portable CPU release macOS arm64 wheel installed outside the repository. The pinned default potion-base-8M model is local (29.49 MiB for the three required files), with files already in OS cache. One thread, five repeats per cache setting, fresh Finder per repeat, 1,000 synthetic English descriptions of about 30 words with integer IDs, 1,000 partitions (one target each). Input generation and construction are outside timing. Full add includes local model loading and conversion. Each repeat warms 20 queries, then times 1,000 complete search calls with reused selected text, new query IDs and changed unselected metadata, threshold 0.8, output limit 10.

Median of five search p95 values: 0.0721 ms with cache disabled, 0.0218 ms with the default 4 MiB cache. Median full add including local model load is 59.2 ms without cache and 59.9 ms with cache; this small difference is within measurement noise. The cache retains 1,000 entries in 1,475,218 accounted bytes, with 1,000 misses, 1,020 hits and zero evictions in the final repeat. All queries return the expected record without truncation.

This workload isolates semantic-text reuse rather than large-partition retrieval. It does not imply that cache hits accelerate every 100k search by the same ratio or improve quality. Process peak RSS across both settings/repeat lifetimes is recorded in the raw file; cache bytes are a conservative retained estimate, not an isolated RSS delta or peak bound. The measured compressed wheel is approximately 1.672 MiB; no dependency is added. The real-model U-012 threshold, held-out/group metrics, candidate/oracle results and critical regression measurements remain exactly equal. See the [cache contract](../guide/embedding-cache.md).

## Explicit snapshots (U-014)

[Reproduction script](../../benchmarks/performance/python_snapshots.py),
[raw measurements](../../benchmarks/performance/snapshots-macos-arm64.json). On
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
whole-batch versus chunked-row oracle. The [persistence contract](../guide/persistence.md)
documents Windows directory-sync and retained-generation limitations. Other
platform results await native CI; no release is published by U-014.

## U-015 mutations and compaction

[Reproduction script](../../benchmarks/performance/python_mutations.py) and
[raw samples](../../benchmarks/performance/mutations-macos-arm64.json). Release abi3 wheel,
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
mutation's compaction. See the [mutation contract](../guide/mutations.md).

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

[Protocol and vector recall](../../benchmarks/spikes/ann-backends/README.md),
[script](../../benchmarks/performance/python_ann.py) and
[raw samples](../../benchmarks/performance/ann-macos-arm64.json). Release abi3
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

[Script](../../benchmarks/comparison/scale.py), [raw results](../../benchmarks/performance/all-pairs-macos-arm64.jsonl), U-003 protocol: synthetic company names in one partition, one record in ten a near-duplicate with a typo, `min_score=0.8`, pairs turned into removals by SemHash's rule inside the timed section. Apple M2 (4 performance and 4 efficiency cores), 16 GiB, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0, release build, potion-base-8M at `bf8b0566` in OS cache, one run per line, one process at a time. Wall time includes model loading; CPU time covers deduplication on every thread.

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

Every configuration removes the same records as before. For reference, SemHash 0.5.0 takes 1.0 s, 2.5 s and 14 s on the same data with every core ([U-003](../../benchmarks/comparison/README.md)).

- **Half the comparisons.** An all-pairs query now scans only the records after it, in the lexical verification and the dense semantic scan: one thread is about twice as fast.
- **Threads.** Lexical reports scale to 5.1× on 8 threads at 100,000 records. Dense semantic reports stop at 1.9× there: each query streams every later vector from memory, up to 100 MB at 100,000 records, so memory bandwidth, not arithmetic, bounds them from about 4 threads. [ADR 0019](../adr/0019-blocked-dense-all-pairs.md) measures this cause in isolation and compares blocks of queries with each vector read.
- **HNSW** stays slower than the exhaustive scan up to 30,000 records; its graph is still built on one thread.
- **Small calls.** With `threads=1`, a lexical index creates no pool: `find` of 20 records takes 483 µs against 510 µs before, and `group` 83 µs against 85 µs (median of five runs of 2,000 calls). With `threads=4`, creating the index's pool adds about 25 µs to `find` and 50 µs to `group`.

## Blocked dense all-pairs (ADR 0019)

**Cause, in isolation.** [`dense_bandwidth`](../../crates/univo-engine/examples/dense_bandwidth.rs) times the scoring cosine of 256-dimensional vectors, query by query or in blocks of 64 queries sharing each row read, with the vectors in cache (1 MB) or in memory (100 MB, a 100,000-record partition). [Raw output](../../benchmarks/performance/dense-bandwidth-macos-arm64.csv); best of three, Apple M2 on battery at full charge, low-power mode off.

| Vectors | Mode | 1 thread | 4 threads | 8 threads | Read on 8 threads |
| --- | --- | --- | --- | --- | --- |
| 1 MB | query by query | 19.2 ns/dot | 5.7 ns | 4.1 ns | 253 GB/s (cache) |
| 100 MB | query by query | 20.5 ns | 8.3 ns | 10.1 ns | 102 GB/s |
| 1 MB | blocks of 64 | 10.9 ns | 3.3 ns | 2.4 ns | 6.6 GB/s |
| 100 MB | blocks of 64 | 11.3 ns | 3.3 ns | 2.6 ns | 6.1 GB/s |

Query by query, the in-memory scan stops near the M2's nominal 100 GB/s from 4 threads, and 8 threads are slower than 4; in cache the same work scales to 4.7×. In blocks, cache and memory give the same times: the scan no longer depends on memory bandwidth.

**End to end.** U-003 workload (synthetic names, one partition, `min_score=0.8`, [script](../../benchmarks/comparison/scale.py)), model load included, same conditions; [raw results](../../benchmarks/performance/blocked-dense-macos-arm64.jsonl).

| Records | Before, 1 thread | Blocked, 1 thread | Before, 8 threads | Blocked, 8 threads | CPU, 8 threads | Peak RSS, 8 threads | SemHash 0.5.0 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 10,000 | 1.4 s | 0.76 s | 0.36 s | **0.23 s** | 1.2 s | 131 MiB | 1.0 s |
| 30,000 | 11.7 s | 5.4 s | 4.1 s | **1.43 s** | 9.2 s | 179 MiB | 2.5 s |
| 100,000 | 126 s | 57 s | 66.5 s | **13.7 s** | 100 s | 347 MiB | 14.0 s, 643 MiB |

Every run removes the same records as before. Univo's search is exact; SemHash's index is approximate.

**Memory of similar data.** Blocks hold at most 2^20 candidates and split when they would hold more. 100,000 records with identical vectors, `groups(max_pairs=1)` on 8 threads, peak RSS above the index: 1,385 MiB with 16 semantic fields before the bound, 129 MiB with it; 153 MiB and 62 MiB with one field.

The all-pairs harness [`all_pairs`](../../crates/univo-engine/examples/all_pairs.rs) gives 11.2 ns per pair on one thread at 20,000 records, against 26.5 ns for the same kernel left to the compiler's vectorizer.

## Multilingual default model (ADR 0021)

The default model now has 128 dimensions instead of 256 ([ADR 0021](../adr/0021-multilingual-default-model.md)). Model-level budgets, release build, same reference machine:

| Measure | potion-base-8M | New default |
| --- | --- | --- |
| Load, verify and encode one text, median of 5 | 18 ms | 60 to 95 ms |
| 20,000 descriptions, 1 thread / 8 threads | 0.95 s / — | 0.82 s / 0.27 s |
| Exhaustive top-10 over 100,000 vectors, one thread | 2.5 ms | 1.2 ms |
| Search p95 at 100,000 records, one partition / ten / a group of 10,000 | 10.0 / 2.3 / 8.5 ms | 8.5 / 1.65 / 6.6 ms |
| Indexes for that workload | 156 MiB | 92 MiB |

Loading is slower because the tokenizer and the vocabulary are larger (105,879 tokens); it happens once per Finder, at the first semantic use.

**End to end.** Same U-003 workload as above, every core, model load included. Both tools use the new default model. Release wheel installed outside the tree, SemHash 0.5.0, on battery at 96%, low-power mode off; [raw results](../../benchmarks/performance/default-model-macos-arm64.jsonl).

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

## Performance audit fixes

Apple M2, release wheels outside the source tree, `main` and the change alternated. Measured on battery from 35% down to 16%, low-power mode off.
- **Same results.** Five `find` and `group` configurations (lexical and semantic, minimum scores 0.6 to 0.9) return identical pairs, scores to the bit, explanations and groups. Snapshots are byte for byte identical.

| Measure | `main` | Change |
| --- | --- | --- |
| Semantic all-pairs, 100,000 names, every core | 7.45–7.76 s, 52–55 s CPU, 301 MiB | **4.75–4.84 s**, 32–33 s CPU, 270 MiB |
| Lexical all-pairs, 100,000 names, every core | 24.9–25.0 s, 182 MiB | 24.4 s, **159 MiB** |
| Add 100,000 records with IDs: peak RSS | 157.5 MiB | **135.0 MiB** |
| Save a 100,000-record semantic snapshot | 0.28–0.38 s | **0.20 s** |
| Load it | 0.29 s, 460 MiB | **0.23 s**, 403 MiB |
| 2,000 HNSW searches over 30,000 records | 1.20 s | 1.10 s |
| Search p95, group of 10,000 identical records | 6.45 ms | 6.22 ms |
| `find`, `group` and search at 30,000 records | — | within noise (±3%) |

With the same model, SemHash 0.5.0 takes 8.72 s on the 100,000-name semantic run ([multilingual default](#multilingual-default-model-adr-0021)).

- **Blocked dense scan.** For each group of eight queries and each row, the products of the first half of the dimensions come first. The rest is bounded by the product of the norms of the remaining dimensions (Cauchy–Schwarz), plus a rounding margin of `4·n·ε`.
  - **Skip.** When no query of the group can reach the lane threshold, the row is skipped.
  - **Otherwise.** The products continue from the same partial sums, so similarities are bit for bit those of the full kernel, and the dense lane keeps returning what the oracle returns.
- **Scoring without explanations.** Candidates are scored without building per-field explanations and with one vector buffer per query.
  - **Search** explains only the `limit` matches it keeps.
  - **`find`** feeds each worker's bounded heap directly and explains the kept pairs once the report is complete.
  - **`group`** explains none.
- **Exact buckets.** A bucket holding one ID stores it inline; a tree is built for the second, instead of one tree per distinct record.
- **Snapshots.** Records and vectors are written and read a few bytes at a time; they are now hashed and written in 64 KiB chunks. The digest and the format are unchanged.
- **HNSW.** The search explores `max(k, ef_search)` nodes and returns the first `k`. Every request up to `ef_search` is a prefix of one traversal, which is now done once instead of once per doubling of `k`.
- **Lexical all-pairs.** Posting lists hold slots in increasing order. The entries below the query's slot are skipped by binary search, yet still counted against `max_entries`, so truncation is unchanged.
- **Exhaustive all-pairs.** Each query streams the later slots of its partition directly, instead of copying the whole partition: `O(n²)` slot copies fewer over a report. Candidate counts are unchanged, since earlier slots were excluded anyway.
- **Dense lane of one search.** Slots are appended to the candidate list without the sort of `within`, as the list is sorted once anyway. The public `within` still sorts.
- **No prefix.** Below 16 dimensions, the kernel computes the products directly, with no bound and no norm.

Same machine, the change as first published against the change with the review fixes, alternated three times. It ran partly on battery and the reference itself varied from 5.1 to 8.9 s on one case, so only paired runs compare.

| Measure | Before review fixes | After |
| --- | --- | --- |
| Semantic all-pairs, 100,000 names | 5.06, 8.89, 8.03 s | 6.82, 8.24, 7.50 s |
| Semantic groups, 100,000 names | 6.92, 7.97, 7.47 s | 7.10, 7.93, 7.48 s |
| Exhaustive lexical all-pairs, 10,000 names | 5.87, 3.08, 3.04 s | 4.04, 2.95, 2.95 s |
| Exhaustive semantic all-pairs, 10,000 names | 0.86, 0.69, 0.67 s | 0.73, 0.58, 0.68 s |
| 2,000 dense searches over 100,000 names | 4.05, 3.18, 3.14 s | 3.60, 3.33, 3.30 s |

- **Within noise.** None of these changes is measurable here: the copies and the sort were small next to the scoring they precede. They remove work that grows with the partition.
- **Run order.** Alternating each pair in the same order favoured the first run by about 3% as the machine heated. In ABBA order, the semantic all-pairs run takes 8.77 s after the review fixes and 8.91 s before (means of four, with thermal throttling on both sides).
- **Kernel call.** A first version called `dots` for fields without a prefix next to `dots_reaching` in the scan loop. Two inlined kernels in that loop slowed the 100,000-name run from 5.0 to 8.3 s; the case now lives in `dots_reaching`.
- **Not kept.** Norms precomputed once per report, instead of once per block of 64 queries, were measured neutral (5.0–5.4 s on both sides): a row's norm costs a 64th of the products of a block. Same identical outputs as above.

## Lexical signatures

Each record of a lexical field keeps a 128-bit signature of its trigrams. Before the scoring function verifies a candidate, upper bounds of its Jaccard index and edit similarity, derived from the signatures, must allow the threshold ([lexical candidates](../design/lexical-search.md#how-the-search-stays-complete)). The search stays exact.

Apple M2 on battery, release wheels outside the source tree, one partition, minimum score 0.8, every core. The change before and after alternated in ABBA order:

| 100,000 names | Before | After | Speed-up |
| --- | --- | --- | --- |
| `find`, synthetic names | 30.8 s, 176–184 s CPU | **4.0–4.4 s**, 24–25 s CPU | ×7.3 |
| `group`, synthetic names | 33.8–33.9 s | **5.0–5.2 s** | ×6.6 |
| `find`, ROR names | 24.7–28.5 s, 152–158 s CPU | **12.1–13.0 s**, 73–78 s CPU | ×2.1 |
| `group`, ROR names | 26.6–28.4 s | **12.3–12.9 s** | ×2.2 |

- **Same results.** Six `find` and `group` configurations return identical pairs, scores to the bit, explanations, candidate counts and groups: synthetic and ROR names, minimum scores 0.8 over 100,000 records, 0.6 over 30,000 and 0.5 exhaustively over 5,000. Peak memory stays within 4 MiB.
- **ROR names.** These are 100,000 distinct names, labels, aliases and acronyms drawn from the ROR v2.14 dump (seed 0). They gain less than the synthetic names: there are 85,063 real matches to verify, and many short acronyms, whose candidates come from length buckets.
- **Cost.** The signatures take 16 bytes per record and lexical field. They add no dependency and change neither the scores, nor the snapshot format, nor the comparator, which still verifies every candidate the bounds admit.
- **Origin.** The approach comes from an external prototype: 3.50–3.78 s for the native engine on synthetic names, against 29.1–32.6 s.

## Lexical character counts

The summary of each short text adds its character counts in 32 hashed buckets to its trigram signature. The bag distance of two texts, the larger of the characters each has in excess, bounds their edit distance from below. It completes the q-gram bound, which edit distances let through on names that share words ([lexical candidates](../design/lexical-search.md#how-the-search-stays-complete)).

**Profile on ROR names.** 100,000 ROR names, minimum score 0.8, on `main`.
- **Where the time went.** About 70% of the busy time was scoring candidates and 20% reading postings; the signature check took 2%.
- **Candidates.** The signatures rejected 109 million candidates. 67 million more were scored for 84,000 matches.
- **What a stronger bound would remove.** Of the scored pairs that did not match:
  - a 256-bit signature would have rejected 78%;
  - the exact trigram counts, 94%;
  - the 128-bit signature with character counts in 16 buckets, 91%;
  - with 32 buckets, 97.8%;
  - with exact character counts, 99.2%.
- **Choice.** 32 buckets with the existing signature: 48 bytes per record, and no second pass over the trigrams.

Apple M2 on battery, release builds, one partition, minimum score 0.8, every core, `main` and the change alternated in ABBA order:

| 100,000 names | `main` | Change | Speed-up |
| --- | --- | --- | --- |
| `find`, ROR names, wheel | 8.9–9.2 s, 63 s CPU | **2.2–2.3 s**, 15 s CPU | ×4.0 |
| `group`, ROR names, wheel | 8.9–9.0 s | **2.4 s** | ×3.7 |
| `find`, synthetic names, wheel | 3.22 s, 23 s CPU | **1.86–1.89 s**, 13 s CPU | ×1.7 |
| `group`, synthetic names, wheel | 3.34–3.39 s | **2.08–2.09 s** | ×1.6 |
| `find`, ROR names, native engine | 9.6–10.7 s | **2.26–2.31 s** | ×4.4 |

- **Same results.** Six `find` and `group` configurations return identical pairs, scores to the bit, explanations, candidate counts and groups: synthetic and ROR names, at 0.8 over 100,000 records, 0.6 over 30,000 and 0.5 exhaustively over 5,000. The native engine's whole report hashes the same.
- **Memory.** Peak memory grows by 1 to 3 MiB at 100,000 names.
- **One search.** One thread, at 0.7, takes 1.21 ms instead of 11.3 ms at the median.
- **Index build.** It takes 35 ms instead of 29 ms.
- **Reproduce.** `cargo run --release -p univo-engine --example lexical_all_pairs -- names.txt 100000 8` prints the time and a hash of the report. The ROR sample is 100,000 distinct names from the ROR v2.14 dump, shuffled with seed 0.

## Distinct words (ADR 0023)

Release builds, Apple M2, `main` and the change alternated.

| Measure | `main` | Distinct words |
| --- | --- | --- |
| Compare 1,000,000 standard record pairs, alone | 325 ns per pair | 371 ns per pair |
| Prepare 100,000 standard records, alone | 1,175 ns per record | 1,255 ns per record |
| Lexical all-pairs, 100,000 ROR names, wheel | 2.06–2.09 s | 2.15–2.17 s |
| Semantic all-pairs, 100,000 synthetic names, wheel | 4.49–4.56 s | 4.46–4.52 s |
| Lexical all-pairs, 100,000 synthetic names, wheel | 1.91–1.97 s | 1.91–1.93 s |

- **Where it costs.** A 64-bit summary of content words, kept per text, settles most pairs without reading words. The standard comparison workload draws names from 23 words, so 14% of its pairs share one and take the word path.
- **Dead ends.** Splitting words on each comparison, word arrays on the stack and lazy article lookups each left the comparison at 460–490 ns.
- **Edit distance.** Variants are checked by a bounded edit distance instead of `edit_similarity`, whose ASCII path clears a 1 KiB table per call; a test checks that both agree.

