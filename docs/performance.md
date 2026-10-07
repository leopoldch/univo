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
