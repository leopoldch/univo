# 0001. Project name and license

- Status: Accepted
- Date: 2026-10-07
- Roadmap: U-001

## Context

Implementation work (U-005 onwards) needs a distribution name for the Python package and the Rust crates, and a license that every dependency and model must be compatible with. The specifications propose the name Univo and the MIT license without confirming availability.

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

Re-checked on 2026-10-13T23:51Z, before choosing where to host a project model:

| Registry | Query | Result |
| --- | --- | --- |
| PyPI and TestPyPI | `univo` | 404, not registered |
| crates.io | the five crate names above | 404, not registered |
| Hugging Face | namespace `univo` | taken: an empty user account `UnivO` (namespaces are case-insensitive), unrelated |
| Hugging Face | `univo-models`, `univo-ml`, `univo-project`, `univoai` | free |
| GitHub | organization `univo` | unchanged owner; its two repositories and `univo.app` are blockchain indexing and explorer software |
| Web search | `"univo" trademark software` | no registered software mark found; nearest results are distinct names (Univa, Univention) |

The blockchain indexer is open-source data tooling, but it neither detects duplicates nor publishes a `univo` Python package or crate. The name is kept; the collision is recorded so that documentation and search results stay unambiguous.

The MIT license was checked against the same lock file. The workspace crates reach 144 third-party packages through normal dependencies, and every one is permissive: MIT, Apache-2.0, or a choice among MIT, Apache-2.0, BSD-2-Clause, Zlib, BSL-1.0, Unlicense and Unicode-3.0. The only Apache-2.0-only packages linked into the wheel are `tokenizers`, `safetensors`, `esaxx-rs` and `spm_precompiled`. None of the four ships a NOTICE file, so redistribution needs their license text, which the wheels already must include, and no NOTICE. Apache-2.0 model weights downloaded at run time do not change the license of the code. Their attribution belongs in the model card of any modified copy the project publishes.

## Consequences

- Every dependency and every default model must have an MIT-compatible license; this check is part of each dependency record and of the model spike (U-004).
- Wheels must ship the license texts of statically linked Rust dependencies (U-019).
- Documentation must not link to the `univo` GitHub organization as if it belonged to the project.
- The `univo` Hugging Face namespace is unavailable. A model published by the project uses another namespace, chosen and pinned by the default-model decision; documentation must not link to `huggingface.co/univo`.

## Revisit when

- PyPI or crates.io rejects or reserves the name before the first publication.
- A trademark or naming conflict is reported in the duplicate-detection or data tooling space, or the `univo` blockchain indexer starts publishing to PyPI or crates.io.
- A required dependency or model forces a different license.
