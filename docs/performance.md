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
