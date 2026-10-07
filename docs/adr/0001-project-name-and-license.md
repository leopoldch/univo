# 0001. Project name and license

- Status: Accepted
- Date: 2026-10-07
- Roadmap: U-001

## Context

Implementation work (U-005 onwards) needs a distribution name for the Python package and the Rust crates, and a license that every dependency and model must be compatible with. The specifications propose the name Univo and the MIT license without confirming availability ([01-product.md §4](../../specs/01-product.md)).

## Decision

- Python distribution and import name: `univo`.
- Rust crates: `univo-core`, `univo-engine`, `univo-embed` and `univo-python`, with internal crates added only when their roadmap block needs them. A facade crate named `univo` may be published later; the name is currently free.
- Source repository: `github.com/leopoldch/univo`.
- License: MIT for all code in this repository.

The names are not reserved yet. They are claimed at the first publication, not earlier, to avoid squatting placeholder packages.

## Alternatives

- Apache-2.0 or dual MIT/Apache-2.0, the usual Rust ecosystem choice. Apache-2.0 adds an explicit patent grant but requires NOTICE handling in redistributed wheels. MIT keeps the redistribution notice minimal; dual licensing can still be added later with every contributor's consent.
- A different name. Not pursued: the registry names are free and the main collisions listed below are in unrelated domains.

## Evidence

Checked on 2026-10-07T13:33Z:

| Registry | Query | Result |
| --- | --- | --- |
| PyPI | `https://pypi.org/pypi/univo/json`, `https://pypi.org/simple/univo/` | 404, not registered |
| crates.io | `univo`, `univo-core`, `univo-engine`, `univo-embed`, `univo-python` | 404, not registered |
| GitHub | organization `univo` | exists, owned by an unrelated party |
| GitHub | repository search `univo` | unrelated projects (`univocity-parsers`, `UniVoice`); none about deduplication |

PyPI may still refuse a name it considers confusingly similar to an existing project; this can only be confirmed by the first upload.

## Consequences

- Every dependency and every default model must have an MIT-compatible license; this check is part of each dependency record and of the model spike (U-004).
- Wheels must ship the license texts of statically linked Rust dependencies (U-019).
- Documentation must not link to the `univo` GitHub organization as if it belonged to the project.

## Revisit when

- PyPI or crates.io rejects or reserves the name before the first publication.
- A trademark or naming conflict is reported in the duplicate-detection or data tooling space.
- A required dependency or model forces a different license.
