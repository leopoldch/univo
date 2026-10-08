# Pairwise scoring

How two prepared records are compared, field by field, into a score, a coverage and a list of contradictions. This implements [specs/03-matching.md §10](../specs/03-matching.md) with the lexical signals of U-008; semantic similarity joins with the model backend (U-009). The rules are versioned by `SCORING_VERSION`: any change in a similarity, a signal or a score increments it.

Scores rank pairs. **They are not probabilities**, and no threshold is defined here: thresholds are calibrated on a separate dataset (U-012).

## Comparing two records

`compare(schema, left, right)` takes two records prepared with the same schema.

- Records of different partitions are never compared: the result is `None`.
- Records prepared with another schema are refused with `state.schema_mismatch`.

Otherwise each field yields a similarity in `[0, 1]`, or no similarity when it gives no evidence.

| Field | Similarity | Signal |
| --- | --- | --- |
| absent, null or blank on at least one side | none; `incomplete` when the other side has a value | `missing` |
| text, lexical or hybrid mode | the larger of the edit similarity and the trigram similarity | `edit_distance` or `trigrams` |
| text, semantic mode | none until embeddings are available (U-009) | `semantic` |
| exact, scalar or list | 1 when equal after normalization, 0 otherwise, with a contradiction | `exact` |
| exact, object | equal keys / keys present on both sides; a differing key is a contradiction, a key on one side only makes the field `incomplete` | `keys` |

A contradiction on a field declared discriminant (`conflict = "reject"`) is a **veto**: the pair must not match, whatever its score. A null value inside an object is unknown, like an absent key: it never contradicts.

## Lexical signals

Both work on the normalized text: NFC, case folded unless the field disables it.

- **Edit similarity**: `1 - levenshtein(a, b) / max(len a, len b)`, over characters.
  - It is computed only when both strings have at most 64 characters, by the bit-parallel algorithm of Myers (in Hyyrö's formulation for the global distance), which keeps a column of the dynamic-programming matrix in one machine word.
  - It catches typos in short strings, where trigrams are least informative.
- **Trigram similarity**: Jaccard index of the sets of character trigrams of the text padded with two spaces at each end.
  - Trigrams are extracted once, when a record is prepared, and kept sorted, so a comparison is a linear merge.
  - Only lexical and hybrid text fields keep them, which is why the text mode belongs to the schema's preparation fingerprint.

## Score and coverage

Over the fields `O` that give a similarity, with weights `w`:

```text
score    = Σ_{j ∈ O} w_j s_j / Σ_{j ∈ O} w_j
coverage = Σ_{j ∈ O} w_j / Σ_j w_j
```

When no field gives evidence the score is absent, never 1. Coverage tells how much of the schema supported the score: a semantic field before U-009, a missing value or a one-sided object lowers it.

## Explanations

The comparison lists, for each field in schema order, its similarity, its signal, whether it contradicts, whether the contradiction vetoes, and whether evidence is one-sided. Exact matches additionally report the normalization steps they needed (see [duplicate definitions](duplicate-definitions.md)).

## Validation

- `crates/univo-core/tests/comparison_corpus.rs`:
  - every pair of the critical corpus respects the hard rules: no comparison across partitions; a veto exactly when a discriminant field contradicts; `compatible` pairs incomplete and not vetoed; `exact` pairs scored 1;
  - the score is symmetric and deterministic;
  - its AUC on the corpus (0.763 at U-008) is guarded against regressions.
- `crates/univo-core/tests/lexical_properties.rs`: edit similarity and trigram Jaccard against naive reference implementations.
