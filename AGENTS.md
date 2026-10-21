# Repository instructions

## Purpose and source of truth

Build Univo: a fast, local duplicate-detection library with a reusable Rust core and a thin Python interface. Before changing a component, read its pages in [docs](docs): the guide for behavior, the reference for contracts such as scoring, errors and stability, and the design pages for internals. Keep them in sync with the code.

## Non-negotiable design constraints

- Keep matching, indexing and decision logic in Rust. Python provides ergonomic input/output conversion, not a second scoring implementation.
- Run inference locally. Never send records to a remote model API. No telemetry by default and no user data in download requests or routine logs.
- Keep import and Finder construction free of model loading and network access. Resolve the default model lazily, cache it and support a strict offline path.
- Minimize total installed size, model size, memory and transitive dependencies. Do not equate a short dependency list with a lightweight distribution.
- Preserve exact and lexical baselines. Embeddings are one source of evidence, not a proof of identity.
- Respect partition boundaries, typed values, veto rules and missing-value semantics before making a match decision.
- Never delete or merge external records implicitly. Scores are not probabilities unless a separately validated calibration exists.
- Expose candidate limits, truncation and approximate retrieval. Retain exhaustive search as an oracle and supported mode within its declared resource budget.
- Do not claim that local check_or_add provides database-wide or distributed atomicity.
- Ship precompiled Python wheels for every advertised platform. Rust must not be required to install a supported wheel.

## Implementation workflow

1. Define the task and its acceptance criterion before coding.
2. Keep changes scoped to that task and its necessary integration. Do not add Java bindings, database connectors or automatic merging before their planned milestones.
3. For unresolved choices (model, ANN backend, thresholds, ABI), run a spike or benchmark first, and publish its results with the method ([benchmarks](docs/benchmarks.md), [performance](docs/development/performance.md)).
4. Update affected contracts when changing API behavior, on-disk formats or model compatibility. Do not silently change a target just to make a check pass.
5. Validate the concrete risks introduced by the change and report what was tested, the result and any remaining limitation.

Follow current maintainer instructions for the task; reconcile changes with the documentation. Routine reversible implementation work does not require an extra approval step. Do not publish a release or merge a PR without the maintainer's request.

## Rust and Python boundaries

- Core types and matching logic must not depend on PyO3 or network clients.
- Use explicit typed errors; never let a panic cross the Python FFI boundary.
- Convert inputs in batches. Release the Python runtime during long Rust-only work; do not touch Python objects while detached.
- Avoid per-candidate Python callbacks, unnecessary copies and unbounded caches. Control thread counts and avoid nested thread-pool oversubscription.
- Prefer safe Rust and maintained components. Any unsafe optimization needs a written invariant, tests and a measured benefit.
- Compile distributed binaries for portable CPU targets; any optional instruction-set acceleration needs a tested fallback.

## Validation and performance

- Follow [performance](docs/development/performance.md) for measurement and budgets, [benchmarks](docs/benchmarks.md) for quality, and [releasing](docs/development/releasing.md) for packaging.
- Measure release builds. Report hardware, dataset, threads, versions, cold/warm state, model footprint and end-to-end latency, not only index lookup time.
- Compare approximate search at a measured recall. Report candidate recall separately from final duplicate precision/recall.
- Keep train/validation/test entities separate. Never tune thresholds on the held-out test set.
- Test mutations, snapshots and offline behavior when changing those components. Test installed wheels outside the source tree and without a Rust toolchain.
- Documentation-only changes need link checks and tested examples.

## Dependencies and versioning

For each new dependency, record its purpose, enabled features, license, transitive/native footprint and effect on wheels. Keep optional integrations optional. Pin model revisions and verify downloaded files. Do not execute downloaded model code or use pickle for persistent state.

Schema, normalizer, encoder and format changes must explicitly handle cache/index compatibility. Reject incompatible state instead of silently mixing representations.

## Change reporting

Commit and PR descriptions must explain the concrete change and its validation. Do not append automated contributor attributions or co-author trailers. Keep user data, credentials and unrelated local artifacts out of commits.
