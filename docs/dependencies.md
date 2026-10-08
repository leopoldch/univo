# Dependency register

Every dependency is recorded here with its purpose, enabled features, license, footprint and effect on wheels, as required by [AGENTS.md](../AGENTS.md). Runtime dependencies end up in the wheels; development dependencies do not.

## Runtime

| Crate | Version | Used by | Purpose | Features | License | Transitive | Footprint |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `pyo3` | 0.29.3 | `univo-python` | Python bindings | `macros`, `abi3-py311`; `extension-module` for wheels only | MIT OR Apache-2.0 | `libc`, `once_cell`, `portable-atomic`, `pyo3-ffi`; macros and build config at compile time only | extension of 0.31 MiB stripped, wheel of 0.15 MiB (U-005) |
| `unicode-normalization` | 0.1.25 | `univo-core` | NFC composition and its quick check; embeds Unicode 17.0 | `std` only | MIT OR Apache-2.0 | `tinyvec` (Zlib OR Apache-2.0 OR MIT) | with `caseless`: +177 KiB in a stripped release binary that normalizes (U-006) |
| `caseless` | 0.2.2 | `univo-core` | Unicode default full case folding | none | MIT | `unicode-normalization` | included above |
| `rustc-hash` | 2.1.3 | `univo-core` | Fast non-cryptographic hash for normalization fingerprints | `std` disabled | Apache-2.0 OR MIT | none | negligible |
| `model2vec-rs` | =0.3.0 | `univo-embed` | Static Model2Vec inference from verified bytes | default features off; `local-only`, `fancy-regex` | MIT | `tokenizers` 0.21.4 (Apache-2.0), `safetensors`, `ndarray`, `half`, `rayon`, `serde`, `anyhow`, about 100 crates in all; pure Rust, no C or C++ build, no network crate | +2.68 MB stripped, +1.15 MB compressed on macOS arm64 (U-009) |
| `sha2` | 0.11.0 | `univo-embed` | SHA-256 of pinned and LFS model files | default features off | MIT OR Apache-2.0 | `digest`, `block-buffer`, `cpufeatures` (runtime detection with portable fallback) | included above |
| `sha1` | 0.11.0 | `univo-embed` | Git blob ids of small hub files | default features off | MIT OR Apache-2.0 | shared with `sha2` | included above |
| `serde` | 1.0.229 | `univo-embed` | Reads only the length settings of `tokenizer.json`, skipping the rest | `derive`, `std` | MIT OR Apache-2.0 | already linked through `tokenizers` and `serde_json` | included above |
| `serde_json` | 1.0.151 | `univo-embed` | Hub metadata, cache manifests and the length settings of `tokenizer.json` | default | MIT OR Apache-2.0 | `itoa`, `memchr`, `ryu`, `serde` | included above |

`caseless` was checked against CPython's `str.casefold` on every Unicode code point (0 differences); the check runs in `crates/univo-core/tests/unicode_oracle.rs`. See [ADR 0003](adr/0003-unicode-normalization.md).

`univo-embed` ([ADR 0008](adr/0008-model-download-and-inference-backend.md)) is not yet linked into the Python wheel; U-009's Python part links it. Its 108 linked packages are listed by `cargo tree -p univo-embed -e normal`; their licenses are permissive (MIT, Apache-2.0, BSD-2-Clause, Unlicense, Zlib, BSL-1.0 and Unicode-3.0 options), and their notices ship with the wheels in U-019. `tokenizers` runs batches on Rayon's global thread pool.

`univo-engine` depends only on the local `univo-core` crate (MIT, no additional features). Its index uses standard-library collections and the core's existing fingerprint code. It adds no third-party or native dependency and is not yet linked into the Python wheel.

## Development only

| Crate | Version | Purpose | Features | License |
| --- | --- | --- | --- | --- |
| `criterion` | 0.8.2 | Benchmarks (`cargo bench`) | `cargo_bench_support` only | Apache-2.0 OR MIT |
| `proptest` | 1.11.0 | Property tests | `std` only | MIT OR Apache-2.0 |
| `serde_json` | 1.0.151 | Reading JSON fixtures in core and engine tests | `preserve_order` | MIT OR Apache-2.0 |
| `toml` | 1.1.6 | Reading `benchmarks/budgets.toml` in tests | `parse`, `serde` | MIT OR Apache-2.0 |
| `rayon` | 1.12.0 | One-thread pool in the embedding budget test; already linked through `tokenizers` | default | MIT OR Apache-2.0 |

The engine's dense-insertion budget test reuses this `toml` development dependency. It enables the same features as the core test, adds no new transitive package and does not affect distributed wheels.

The minimum supported Rust version is 1.89: `model2vec-rs` requires 1.88, and the model cache uses the standard library's file locks, stable since 1.89.

Python development tools (`maturin`, `pytest`, `ruff`, `mypy`) are pinned in `uv.lock`; the package has no runtime Python dependency.
