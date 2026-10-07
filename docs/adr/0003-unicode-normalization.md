# 0003. Unicode normalization and case folding

- Status: Accepted
- Date: 2026-10-07
- Roadmap: U-006

## Context

The normalization rules of [docs/duplicate-definitions.md](../duplicate-definitions.md) need NFC composition and Unicode default full case folding, applied to every record before comparison. They must be exactly reproducible, because the critical corpus is labelled with CPython's implementation, and fast, because every inserted or queried record goes through them.

## Decision

- NFC: `unicode-normalization`. Full case folding: `caseless`. Hashing for explanations: `rustc-hash`.
- Fast paths, all exact:
  - ASCII text is lowercased in one vectorized pass, with no Unicode work.
  - Text whose bytes are all below `0xCC` (no character at or above U+0300) is already in NFC, so NFC is skipped after a byte scan.
  - When NFC is needed, only segments around characters at or above U+0300 are composed.
  - Case folding copies the text, lowercases ASCII in place, and visits only non-ASCII characters through a table for code points below U+0300, derived once from `caseless`.
- Hostnames lowercase ASCII only and are recomposed afterwards, so that the function is idempotent (`T` + U+0308 becomes U+1E97).
- Exact matches report which normalization steps were needed per field. Each prepared field keeps a fingerprint of the value before every step that changed it; the explanation keeps the steps up to the first stage where both sides agree.
- `NORMALIZER_VERSION` is incremented whenever any input normalizes differently.
- A hostname ending with more than one dot is rejected, which keeps hostname normalization idempotent on every accepted input.
- Value normalization is iterative, like the validation of limits. The remaining recursive operations on nested values (fingerprints, comparison, deallocation) need less than 64 KiB of stack at the highest accepted depth of 128, in debug builds as in release; a test runs them on a 256 KiB thread.

## Alternatives

- `icu_casemap` and `icu_normalizer`: complete and maintained, but heavier, under the Unicode-3.0 license, and with more transitive crates.
- Own tables generated from the Unicode data files: no dependency, but tables to maintain and a second source of truth.
- Simple case folding (`str::to_lowercase`): misses `ß` to `ss` and ligatures, and disagrees with the corpus oracle.
- Explanations from the union of transforms on both sides: cheaper, but reports "case" for `Café de Flore` written in NFC and NFD although the case is identical.

## Evidence

Apple M2, release build, single thread:

| Measure | Result |
| --- | --- |
| Agreement with CPython 3.14 (Unicode 16.0) on every code point and 1 506 mixed strings | 0 differences |
| `normalize_text`, ASCII name / 120-byte ASCII description | 8.7 ns / 15 ns |
| `normalize_text`, 140-byte French description | 986 ns at first, 150 ns after the fast paths |
| `prepare`, standard workload (5 % decomposed text, 70 % with `ß`) | 2 970 ns per record at first, 845 ns after |
| Binary size of normalization code | +177 KiB, stripped |

`unicode-normalization` embeds Unicode 17.0; the Unicode version of the `caseless` tables is not documented, but they agree with CPython's Unicode 16.0 case folding on every code point. Unicode's stability policies keep normalization and case folding unchanged for characters already assigned, so the remaining risk is limited to characters added after Unicode 16.

## Consequences

- Python and Rust agree on normalization by construction of the tests, not by sharing code: the corpus oracle in `tests/corpus` and the CPython fixtures in `tests/fixtures/unicode` are independent of the Rust implementation.
- A dependency update that changes any output fails the oracle tests and requires a `NORMALIZER_VERSION` increment.
- Every prepared field carries 24 bytes of fingerprints: a prepared field takes 56 bytes, 32 without them. On the standard workload, a prepared record takes about 690 bytes, of which 72 are fingerprints (about 10 %); with 64 fields, fingerprints alone would take 1.5 KiB per record, 154 MB for 100 000 records.

## Revisit when

- A dependency is unmaintained when a Unicode update is needed.
- Profiling of the full pipeline (U-010) shows normalization above 5 % of the end-to-end insertion or search time.
- The index storage format is designed (U-010): store fingerprints only for fields whose transforms are not empty, in a side table, or make explanations optional, if records with many fields make the 24 bytes per field significant.
- Explanations prove unnecessary for most users, which would allow dropping the fingerprints.
