# Architecture decision records

Each record captures one decision, the alternatives considered, the evidence, the trade-offs and the conditions that would reopen it. Records are immutable once accepted; a later decision supersedes an earlier one by adding a new record and updating the status line of the old one.

| ADR | Title | Status |
| --- | --- | --- |
| [0001](0001-project-name-and-license.md) | Project name and license | Accepted |
| [0002](0002-python-abi-and-wheel-strategy.md) | Python ABI and wheel strategy | Accepted |
| [0003](0003-unicode-normalization.md) | Unicode normalization and case folding | Accepted |

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
