# Critical corpus

Deterministic pairs that cover the critical branches of duplicate detection ([specs/08-validation.md §20](../../../specs/08-validation.md)). Labels follow [docs/duplicate-definitions.md](../../../docs/duplicate-definitions.md). All records are synthetic; real organization names appear only as recognizable examples.

- Version: critical-v1 (213 cases).
- Review status: labels proposed on 2026-10-07; maintainer review pending.
- Source of truth: [tools/build_critical_corpus.py](../../../tools/build_critical_corpus.py). Edit the tables there and regenerate; CI fails if `cases.jsonl` is stale.

## Purpose and limits

This corpus is a **regression gate**, not a tuning set. It must never be used to choose weights or thresholds: calibration uses a separate dataset split by entity (U-012). It is small, so a perfect score here does not demonstrate the precision targets of the quality benchmark.

Hard gates on this corpus once the engine exists:

- zero matches on `constraint: "veto"` and `constraint: "partition"` cases;
- every `expected_kind: "exact"` case reported as `exact`;
- every `invalid` case rejected with `expected_error`.

The other `duplicate` and `distinct` labels feed precision and recall per family.

## Format

One JSON object per line:

| Key | Meaning |
| --- | --- |
| `case_id` | `<family>-<NNN>`, stable |
| `profile` | `strings`, `organizations`, `products` or `configurations` |
| `family` | difficulty family from the validation specification |
| `lang` | language of the text, `mixed` for a French/English pair |
| `label` | `duplicate`, `distinct` or `invalid` |
| `constraint` | `veto`, `partition` or `null`; only on `distinct` cases |
| `expected_kind` | `exact` when the pair must be an exact match, otherwise `null` |
| `expected_error` | error name for `invalid` cases, otherwise `null` |
| `left`, `right` | the two records, as given to the API |
| `rationale` | why the label holds |

Absent keys and `null` values are different on purpose. Object key order is preserved in the file; readers must not rely on it for equality, but the `ordering` family uses it to check that the engine does not either.

## Coverage

| Family | Cases | | Family | Cases |
| --- | --- | --- | --- | --- |
| typo | 24 | | missing_value | 16 |
| semantic_competitor | 15 | | unicode | 16 |
| paraphrased_description | 15 | | ordering | 11 |
| homonym | 16 | | tenant | 15 |
| negation | 16 | | dense_series | 12 |
| critical_parameter | 15 | | short_string | 17 |
| version_environment | 15 | | adversarial | 10 |

Labels: 86 duplicate, 120 distinct, 7 invalid. Constraints: 29 veto, 12 partition. Languages: 156 English, 47 French, 3 mixed, 7 other.
