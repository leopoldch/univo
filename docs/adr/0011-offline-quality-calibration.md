# 0011. Offline calibration and experimental quality scope

- Status: Accepted for the benchmark protocol; no production calibration
- Date: 2026-10-11
- Roadmap: U-012

## Context

Finder exposes an uncalibrated score threshold of 0.8. The critical corpus is a regression set and must not select thresholds. A separate dataset must keep entities disjoint, preserve missing evidence in metrics and distinguish candidate generation from final decisions. The maintainer requires a small, fast local distribution and a default model download under 100 MiB.

## Decision

Use the pinned CC0 ROR names/alias dataset as an initial offline name benchmark, with hash-based entity splits and explicit within-split negative sampling. Reserve train, select a threshold on validation only, then evaluate the held-out test. The evaluator calls the installed Rust-backed Finder; the standard library handles source parsing and metrics. No runtime dependency or scoring version changes.

Retain generic-v1, the default 0.8 convenience threshold and the current proposed English model. None of the tested name comparators meets the declared quality gates. Dataset-specific thresholds are benchmark output, not shipped profiles or probabilities. No automatic identity decision is introduced. The multilingual reference remains an explicit option, with no increase of the 100 MiB default-model target.

## Evidence and alternatives

The [protocol, raw results and corpus manifest](../benchmarks.md#name-quality) compare exact normalized names, lexical matching and three local static models. On 2,514 held-out name pairs, the highest measured recall is 17.9% with 96.8% precision, far below the 90% recall / 98% precision targets. The smaller 2M model gives 17.7% recall with 7.86 MiB of files versus 29.49 MiB for 8M; this single narrow dataset does not establish a replacement default. The multilingual reference uses 506.38 MiB of required files and provides no convincing quality gain here.

- Tune the critical corpus or test: rejected because it leaks evaluation data.
- Lower the default threshold after seeing test failures: rejected; more candidates do not establish better identity decisions.
- Ship a name-only calibrated profile: rejected while validation precision fails to transfer to test and alias recall stays low.
- Increase the model budget: no measured benefit justifies the size/RSS cost.
- Add sklearn or a Python matching implementation: unnecessary; rank metrics and threshold selection are offline tooling, while scoring and retrieval stay in Rust.

## Validation and limits

Tests compare AUC with an independent pairwise oracle, cover tied thresholds, insufficient support, abstention, missing evidence, source determinism and entity leakage. The real Finder runs all critical hard gates with lexical and a tiny local semantic model. Real-model measurements also show zero hard-gate failures, oracle disagreements or truncations.

The registry provides identity labels, not independently double-reviewed duplicate annotations. Negatives are sampled; one eligible target per query does not test large-index candidate recall. Pairwise Wilson intervals do not account for shared entities. A public out-of-domain dataset, reviewed ambiguous pairs, stronger FR/cross-language support, SemHash comparison and full semantic Finder performance remain required before production quality claims or release.

## Revisit

After those measurements, decide whether a narrowly supported profile meets gates, whether structured identity evidence changes the results, or whether a compact multilingual artifact improves the target workload within the unchanged budget. Replacing the default model requires a separate ADR superseding ADR 0007 with measured quality, complete footprint and compatibility evidence.
