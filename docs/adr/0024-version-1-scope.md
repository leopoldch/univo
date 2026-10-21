# 0024. Scope of version 1.0

- Status: Accepted
- Date: 2026-10-14
- Roadmap: U-020, U-021

## Context

The product specification ties 1.0 to stable contracts and to the quality and performance gates. Its validation strategy adds: when the gates fail, publish an experimental alpha **or reduce the supported profiles**, rather than lower a threshold.

**Where the gates stand.** Measured on the named benchmarks, macOS arm64:

| Gate | Target | Measured | Met |
| --- | --- | --- | --- |
| Candidate generation recall | ≥ 99% | Equal to the exhaustive oracle on every test; no truncation | yes |
| Veto and partition violations | 0 | 0 on the 234 critical cases | yes |
| Performance budgets | `benchmarks/budgets.toml` | All pass; SemHash 8.7 s against 4.8 s on 100,000 names | yes |
| Duplicate precision / recall, ROR name aliases | ≥ 98% / ≥ 90% | 96.0% / 40.8% with the default model (ADR 0023) | no |
| Automatic decision precision | ≥ 99.5% | No profile measured | no |

The name-only gates fail for reasons a name cannot overcome: identical names of distinct organizations, translations, acronyms of two to four letters. On 100,000 real ROR names, at most 70% of the matches at 0.9 join one organization, even with the country as a partition.

## Decision

Version 1.0 **reduces the supported profiles** to what the evidence supports.

**Supported and promised.**
- **Candidate search** that is complete for the scoring rule: search and reports return exactly what the exhaustive oracle returns, unless they report a work or output limit, or the explicit approximate profile is used.
- **Explained scores.** Every match comes with per-field similarities, signals and conflicts. Scores rank candidates; they are not probabilities.
- **Hard rules.** Partitions, vetoes, typed IDs, required fields and missing values are always respected.
- **Measured resources.** The budgets of `benchmarks/budgets.toml` hold on the reference machine.
- **Stable contracts.** The Python API, error codes, report formats and snapshot compatibility follow the [stability contract](../reference/stability.md).

**Not promised.**
- **Automatic decisions.** No threshold is a proof of identity. `check_or_add` returns `review` for non-identical matches, and calibrating a threshold is the user's responsibility, on their own labelled data.
- **Name-only quality figures as a guarantee.** The ROR benchmark results are published as measurements with their limits ([quality benchmark](../benchmarks.md#name-quality)).

**What 1.0 is.** It is the Python package `univo`. The Rust crates are its implementation: they are not published on crates.io, and their API may change in any release.

## Alternatives

- **Stay at 0.x until the name gates pass.** Rejected: the remaining gap is not a missing feature but missing information on names alone. A 0.x would delay stable contracts for users whose data has more fields.
- **Lower the duplicate gates.** Rejected by the specification itself: thresholds are not lowered to show more duplicates.
- **Promise a calibrated default threshold.** Rejected: thresholds depend on the data, the fields and the cost of an error. The default `min_score=0.8` is a convenience, documented as uncalibrated.

## Consequences

- **Contracts.** [Stability](../reference/stability.md) lists what 1.x keeps compatible, and how to upgrade.
- **Before freezing.** The Python API was reviewed:
  - `search` takes `limit` by keyword only, like `find` and `check_or_add`;
  - `Match.reasons` uses the documented signal names (`name: edit_distance`), not Rust's debug spelling;
  - `config()` loses its duplicate `effective_threads` key.
- **Release.** The [release checklist](../development/releasing.md#release-checklist) lists what a 1.0 needs that this decision does not settle: wheels for every advertised platform, built and tested by CI, and the final comparative benchmark.

## Revisit when

- **A multi-field benchmark** with labelled duplicates exists. Its gates can then be evaluated, and a decision profile may be supported.
- **Users report quality** on their own data that contradicts the published figures.
