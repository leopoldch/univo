# ADR 0010 — Python Finder and exact evidence without embeddings

Date: 2026-10-11. Status: accepted for U-011.

## Problem and evidence

U-010 provides Rust retrieval and scoring, but Python cannot yet use it through the specified Finder. During integration, the synthetic 13-token test model exposed a baseline gap: two equal `hello world` records on a semantic-only schema have no known token, so their pairwise comparison has no semantic evidence. Both candidate and exhaustive search discarded them despite verified normalized whole-record equality.

## Decision

Python provides frozen schema options and result containers, bounded input conversion and per-instance serialization. Rust retains preparation, retrieval, scoring, IDs, error messages and whole-record equality. Only selected paths are converted, once; model weights are shared through an `Arc`. The model resolver stays lazy. Hybrid defaults retain the lexical rules of ADR 0009. The convenience minimum of 0.8 is explicitly uncalibrated and adjustable; U-012 must evaluate it.

Semantic encoding executes in a Rayon pool bounded by `threads`. Exact/lexical indexes create no pool. Rayon is already linked through the tokenizer, so this adds no transitive package or Python runtime dependency. Imports expose the facade lazily to retain the import budget.

Search now retains a pair whose ordinary score is absent when the Rust whole-record equality oracle verifies it. It gives score 1 and exact field evidence for informative selected values; absent/null/blank fields still give no evidence. This is normalized equality, not identity. The engine exposes `SEARCH_VERSION=1` separately from pairwise `SCORING_VERSION=2`, and both participate in report configuration fingerprints. Partial equality of unknown semantic text is not promoted into semantic evidence; the candidate completeness proof is therefore preserved by the existing whole-record exact lane. The pairwise cosine contract and ADR 0009 remain unchanged.

`find` scores each unordered pair once, retains only bounded top-ranked output and refuses exhaustive tasks exceeding their declared partition pair budget (one million by default). Candidate work and output truncation are distinct. Output limits do not make dense candidate find subquadratic.

## Alternatives

- Accept all unknown-token pairs: no evidence distinguishes them; would produce false matches.
- Treat any equal semantic field as score 1: would change pairwise scoring and require a new equality lane per semantic field. Defer this broader choice to measured model/scoring work.
- Drop verified exact pairs because the encoder lacks vocabulary: contradicts the preserved exact baseline.
- Score or generate IDs in Python: duplicates Rust policy and breaks the reusable-core boundary.
- Global or nested worker pools: make the requested thread budget misleading and can oversubscribe small hosts.

## Validation and trade-offs

Rust and installed-wheel tests cover the synthetic zero-vector model, equality versus distinct unknown texts, exhaustive/candidate agreement, bounded global ranking, partition pair budgets, typed values/IDs, vetoes, failed batches and concurrent serialized adds. Model, wheel, offline and performance checks use the existing tools; local measurements are recorded in [performance](../performance.md).

The Python layer holds a lock across model resolution and computation, favoring consistent generations over concurrent reads. Input/output conversion holds the interpreter; long Rust indexing, encoding and search detach it. Index estimates exclude records and weights, and no total RSS limit is claimed. No snapshot format changes: persistence has not shipped.

## Revisit

Revisit when U-012 calibrates scoring, U-013 provides embedding reuse, U-014 defines persistent fingerprints, or U-017 requires concurrent generations. The multilingual model/download budget remains a separate measured decision; this ADR does not change ADR 0007 or the 100 MiB default-model target.
