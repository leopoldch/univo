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

`univo-embed` ([ADR 0008](adr/0008-model-download-and-inference-backend.md)) is linked into the Python extension since U-009: the macOS arm64 wheel grows from 0.15 MiB to 1.43 MiB (extension of 3.2 MB stripped). Downloads use Python's standard library (`urllib`), imported only when a download starts, so the package still has no runtime Python dependency. Its 108 linked packages are listed by `cargo tree -p univo-embed -e normal`; their licenses are permissive (MIT, Apache-2.0, BSD-2-Clause, Unlicense, Zlib, BSL-1.0 and Unicode-3.0 options), and their notices ship with the wheels in U-019. `tokenizers` runs batches on Rayon's global thread pool.

`univo-engine` depends on the local `univo-core` and `univo-embed` crates (MIT, no additional features); it embeds records through the `Embedder` trait. Its indexes use standard-library collections and the core's fingerprint and dot-product code. Through U-015 it adds no third-party or native dependency beyond `univo-embed`; U-011 links it into the Python wheel. U-016 adds the HNSW dependency described below. The wheel-size effect is recorded in [performance](performance.md).

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


U-011 adds `rayon` 1.12.0 as a direct runtime dependency of `univo-python` to bound semantic encoding by the Finder thread budget (default features; MIT OR Apache-2.0). Rayon and its transitive dependencies were already linked through `tokenizers`: no new third-party or native package is added. The exact/lexical path creates no pool. The Python facade uses only the standard library; it introduces no runtime Python requirement.

U-012 quality tooling uses only the Python standard library and the installed Univo wheel. It adds no runtime, development, native or wheel dependency. ROR names/IDs are an explicitly downloaded CC0 benchmark input, not bundled data or a library dependency; GeoNames location metadata is not exported. See the [corpus manifest and protocol](../benchmarks/quality/README.md).

U-013 adds no dependency or enabled feature. Embedding memoization uses Rust standard-library HashMap/VecDeque, immutable Arc rows and a Mutex, entirely in univo-engine. Python holds only the native cache handle; no Python cache, external runtime, disk cache or bundled model is added.

U-014 reuses `serde` 1.0.229 (derive/std, MIT OR Apache-2.0), `serde_json` 1.0.151 (default, MIT OR Apache-2.0) and `sha2` 0.11.0 (default features disabled, MIT OR Apache-2.0) directly in `univo-engine`; the binding also directly uses `serde_json` for bounded configuration conversion. These packages/features were already linked via `univo-embed`: no new transitive/native package, network client or Python runtime dependency. The core record codec uses only std; OS locks, synchronization and publication use std filesystem APIs. The macOS arm64 compressed wheel measures 1.742 MiB (versus the reviewed U-013 wheel's 1.673 MiB); 112 linked packages remain. Other platform wheel effects are checked in CI, not inferred from this local measurement.

## U-016 runtime addition

`fast-hnsw =1.0.1` (MIT declared in Cargo/README, no LICENSE file in the
crate archive) implements optional HNSW routing. Default features; direct
mandatory dependencies `memmap2` 0.9.11 (MIT OR Apache-2.0), `rand` 0.8.8
(MIT OR Apache-2.0, default plus small_rng). Univo does not use its mmap or
graph persistence APIs. No native C/C++ runtime, network client or worker pool
is introduced. Construction and search use safe Univo code and portable CPUs.

Seven newly locked versions: fast-hnsw 1.0.1, memmap2 0.9.11, rand 0.8.8,
rand_chacha 0.3.1, rand_core 0.6.4, getrandom 0.2.17 (MIT OR Apache-2.0),
and target-specific wasi 0.11.1+wasi-snapshot-preview1 (MIT OR Apache-2.0).
Some names already occur at other versions; the native network audit reports
115 linked crate names versus 112 before. The mandatory code is compiled
into all wheels, while graph allocations are opt-in. macOS arm64 compressed
wheel 1.789 MiB versus 1.762 MiB U-015, approximately +1.5%; no Python runtime
dependency or separate downloaded runtime. Cross-platform CI verifies the
other advertised wheels. Notices remain part of U-019.

The standalone spike additionally uses hnsw_rs 0.3.5 (MIT OR Apache-2.0,
default features disabled) as a reference; it is not a production dependency.
See [ADR 0015](adr/0015-explicit-partition-ann.md) for maturity risks, alternatives
and evidence. Model footprint and default budget do not change.
