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
