# Architecture decision records

Each record captures one decision, the alternatives considered, the evidence, the trade-offs and the conditions that would reopen it. Records are immutable once accepted; a later decision supersedes an earlier one by adding a new record and updating the status line of the old one.

| ADR | Title | Status |
| --- | --- | --- |
| [0001](0001-project-name-and-license.md) | Project name and license | Accepted |
| [0002](0002-python-abi-and-wheel-strategy.md) | Python ABI and wheel strategy | Accepted |
| [0003](0003-unicode-normalization.md) | Unicode normalization and case folding | Accepted |
| [0004](0004-error-model.md) | Error model shared by every binding | Accepted |
| [0005](0005-exact-index.md) | Verified exact-index keys and record identity | Accepted |
| [0006](0006-incremental-exact-postings.md) | Incremental ordering of exact postings | Accepted |
| [0007](0007-default-embedding-model.md) | Default embedding model and model selection | Proposed |
| [0008](0008-model-download-and-inference-backend.md) | Model download, cache and inference backend | Accepted |
| [0009](0009-semantic-evidence-in-scoring.md) | Semantic evidence in pairwise scoring | Accepted |
| [0010](0010-python-finder-and-exact-fallback.md) | Python Finder and exact fallback | Accepted (U-011) |

| [0011](0011-offline-quality-calibration.md) | Offline calibration and experimental quality scope | Accepted (benchmark protocol) |

| [0012](0012-bounded-embedding-cache.md) | Bounded semantic embedding cache | Accepted (U-013) |

| [0013](0013-versioned-snapshots.md) | Explicit versioned snapshots and bounded restore | Accepted (U-014) |

| [0014](0014-id-mutations.md) | Typed ID mutations and amortized compaction | Accepted (U-015) |

## Template

```markdown
# NNNN. Title

- Status: Proposed | Accepted | Superseded by NNNN
- Date: YYYY-MM-DD
- Roadmap: U-XXX

## Context
## Decision
## Alternatives
## Evidence
## Consequences
## Revisit when
```
