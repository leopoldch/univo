# 0002. Python ABI and wheel strategy

- Status: Accepted
- Date: 2026-10-07
- Roadmap: U-005

## Context

Univo ships a native extension that must install without a Rust toolchain on every advertised platform. The number of wheels, the build tooling and the compilation profile determine install size, import time and CI cost.

## Decision

- Bindings: PyO3, built and packaged by maturin, as a mixed project (`python/univo` for the ergonomic layer, `univo._univo` for the extension).
- ABI: CPython stable ABI, `abi3-py311`. One wheel per platform covers CPython 3.11 and later.
- `pyo3/extension-module` is a crate feature enabled only by maturin, so that `cargo build`, `cargo test` and `cargo clippy` work on every platform without custom linker flags.
- Release profile: `opt-level = 3`, fat LTO, one codegen unit, stripped symbols. Panics unwind so that PyO3 converts them instead of aborting the interpreter.
- CPU baseline: the default Rust target of each platform (x86-64 baseline, aarch64 baseline). No `target-cpu=native`; faster instruction sets will require runtime detection and a tested fallback.
- Architectures: x86_64 (amd64) and arm64 (aarch64) are equally supported on Linux glibc (manylinux), macOS and Windows. Every wheel is built and tested on a native runner of its architecture. CI also asserts the architecture of the interpreter process: from the interpreter build platform on Windows (where `platform.machine()` may report the host CPU of an emulated process), from `uname` plus the Rosetta translation flag on macOS, and from `uname` on Linux. Linux user-mode emulation (qemu-user) is not detected; native runner labels are relied upon there.
- The version is defined once in the Cargo workspace; maturin derives the wheel version from it and the extension exposes it as `univo.__version__`.

## Alternatives

- One wheel per CPython version: allows the full non-limited C API but multiplies the build and test matrix by four. Rejected until a measured bottleneck needs it.
- A C ABI (`cdylib` plus `ctypes` or `cffi`): avoids PyO3 but moves conversion and error handling into Python, contrary to the batch-conversion and typed-error requirements.
- setuptools-rust: workable, but maturin has first-class abi3, manylinux auditing and cross-compilation support with less configuration.

## Evidence

Measured on Apple M2, 16 GiB, macOS 27.0, CPython 3.14.7, Rust 1.99.0, PyO3 0.29.3, maturin 1.15.0, release build:

| Measure | Result | Budget |
| --- | --- | --- |
| Compressed wheel (`macosx_11_0_arm64`, abi3) | 0.154 MiB | 30 MiB |
| `import univo`, 50 fresh isolated interpreters | median 0.72 ms, p95 0.85 ms | 20 ms p95 |
| Modules added by `import univo` | `univo`, `univo._univo` | univo only |

Results for the other five platforms come from CI (`wheels`, `wheel-test` and `wheel-test-no-rust` jobs), each on a native runner.

## Consequences

- The ergonomic Python layer may only use the limited API from Rust; conversions are written against PyO3's abi3-compatible paths.
- Free-threaded CPython is not covered by abi3 wheels and stays out of scope.
- Fat LTO lengthens release builds; debug builds remain fast for development.
- Six native runners per CI run, including macOS runners that are billed at a higher rate on private repositories.
- Any architecture-specific optimization must pass on both architectures before merge.

## Revisit when

- The engine exists (U-010): measure whether fat LTO and a single codegen unit improve end-to-end latency enough to justify their build time.
- Batch conversion benchmarks (U-011) show that a non-limited API would reduce end-to-end latency by more than 10 %.
- Free-threaded CPython support becomes a goal.
- The release build time or the cost of native runners becomes a CI bottleneck.
- A hosted native runner for one of the six platforms is withdrawn.
