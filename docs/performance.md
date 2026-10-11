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
