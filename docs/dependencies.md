# Dependency register

Every dependency is recorded here with its purpose, enabled features, license, footprint and effect on wheels, as required by [AGENTS.md](../AGENTS.md). Runtime dependencies end up in the wheels; development dependencies do not.

## Runtime

| Crate | Version | Used by | Purpose | Features | License | Transitive | Footprint |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `pyo3` | 0.29.3 | `univo-python` | Python bindings | `macros`, `abi3-py311`; `extension-module` for wheels only | MIT OR Apache-2.0 | `libc`, `once_cell`, `portable-atomic`, `pyo3-ffi`; macros and build config at compile time only | extension of 0.31 MiB stripped, wheel of 0.15 MiB (U-005) |
| `unicode-normalization` | 0.1.25 | `univo-core` | NFC composition and its quick check; embeds Unicode 17.0 | `std` only | MIT OR Apache-2.0 | `tinyvec` (Zlib OR Apache-2.0 OR MIT) | with `caseless`: +177 KiB in a stripped release binary that normalizes (U-006) |
| `caseless` | 0.2.2 | `univo-core` | Unicode default full case folding | none | MIT | `unicode-normalization` | included above |
| `rustc-hash` | 2.1.3 | `univo-core` | Fast non-cryptographic hash for normalization fingerprints | `std` disabled | Apache-2.0 OR MIT | none | negligible |

`caseless` was checked against CPython's `str.casefold` on every Unicode code point (0 differences); the check runs in `crates/univo-core/tests/unicode_oracle.rs`. See [ADR 0003](adr/0003-unicode-normalization.md).

`univo-engine` depends only on the local `univo-core` crate (MIT, no additional features). Its index uses standard-library collections and the core's existing fingerprint code. It adds no third-party or native dependency and is not yet linked into the Python wheel.

## Development only

| Crate | Version | Purpose | Features | License |
| --- | --- | --- | --- | --- |
| `criterion` | 0.8.2 | Benchmarks (`cargo bench`) | `cargo_bench_support` only | Apache-2.0 OR MIT |
| `proptest` | 1.11.0 | Property tests | `std` only | MIT OR Apache-2.0 |
| `serde_json` | 1.0.151 | Reading JSON fixtures in core and engine tests | `preserve_order` | MIT OR Apache-2.0 |
| `toml` | 1.1.6 | Reading `benchmarks/budgets.toml` in tests | `parse`, `serde` | MIT OR Apache-2.0 |

The engine's dense-insertion budget test reuses this `toml` development dependency. It enables the same features as the core test, adds no new transitive package and does not affect distributed wheels.

`criterion` requires Rust 1.86, which sets the minimum supported Rust version of the workspace.

Python development tools (`maturin`, `pytest`, `ruff`, `mypy`) are pinned in `uv.lock`; the package has no runtime Python dependency.
