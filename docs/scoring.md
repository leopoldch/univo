# Pairwise scoring

How two prepared records are compared, field by field, into a score, a coverage and a list of contradictions. This implements [specs/03-matching.md §10](../specs/03-matching.md) with the lexical signals of U-008 and the semantic similarity of U-010 ([ADR 0009](adr/0009-semantic-evidence-in-scoring.md)). The rules are versioned by `SCORING_VERSION`, now 2: any change in a similarity, a signal or a score increments it.

Scores rank pairs. **They are not probabilities**, and no threshold is defined here: thresholds are calibrated on a separate dataset (U-012).

## Comparing two records

`compare(schema, left, right)` takes two records prepared with the same schema; `compare_with_vectors` also takes the embeddings of their semantic fields, as the engine computes them (see [vector search](vector-search.md)). Without embeddings, semantic fields give no evidence.

- Records of different partitions are never compared: the result is `None`.
- Records prepared with another schema are refused with `state.schema_mismatch`.

Otherwise each field yields a similarity in `[0, 1]`, or no similarity when it gives no evidence.

| Field | Similarity | Signal |
| --- | --- | --- |
| absent, null or blank on at least one side | none; `incomplete` when the other side has a value | `missing` |
| text, lexical mode | the larger of the edit similarity and the trigram similarity | `edit_distance` or `trigrams` |
| text, semantic mode | the cosine of the two embeddings, clamped to `[0, 1]`; none without both embeddings | `semantic` |
| text, hybrid mode | as lexical mode; hybrid fields are not embedded until a combination is validated (U-012) | `edit_distance` or `trigrams` |
| exact, scalar or list | 1 when equal after normalization, 0 otherwise, with a contradiction | `exact` |
| exact, object | equal leaves / leaves present on both sides, compared key by key at every depth; a differing leaf is a contradiction, a key on one side only makes the field `incomplete` | `keys` |

A contradiction on a field declared discriminant (`conflict = "reject"`) is a **veto**: the pair must not match, whatever its score.

Objects are compared key by key, recursively: nested objects present on both sides are descended into, and any other value is a leaf compared by typed equality (a list is a leaf). A null value is unknown at every depth, like an absent key: it never contradicts, it only makes the field incomplete. A key present on one side only counts once, whatever its value. Two empty objects are equal values, but objects whose values are all null give no evidence: `{"port": null}` against `{"tls": null}` has no score and a coverage of 0.

## Lexical signals

Both work on the normalized text: NFC, case folded unless the field disables it.

- **Edit similarity**: `1 - levenshtein(a, b) / max(len a, len b)`, over characters.
  - It is computed only when both strings have at most 64 characters, by the bit-parallel algorithm of Myers (in Hyyrö's formulation for the global distance), which keeps a column of the dynamic-programming matrix in one machine word.
  - It catches typos in short strings, where trigrams are least informative.
- **Trigram similarity**: Jaccard index of the sets of character trigrams of the text padded with two spaces at each end.

`lexical_similarity` is this rule, shared with the lexical index ([lexical candidates](lexical-search.md)), so that candidates and scores never disagree.
  - Trigrams are extracted once, when a record is prepared, and kept sorted, so a comparison is a linear merge.
  - Only lexical and hybrid text fields keep them, which is why the text mode belongs to the schema's preparation fingerprint.

## Semantic signal

A semantic field compares the unit embeddings of its two normalized texts, from the model chosen for the index ([models](models.md)). The similarity is their cosine clamped to `[0, 1]`: opposite and unrelated texts are equally dissimilar. A text without any token the model knows has no embedding, and the field then gives no evidence rather than a made-up similarity.

Hybrid fields are scored lexically. On the critical corpus, adding their cosine to the score, as the larger or the mean of both signals, lowered the ranking quality; the measurements are in [ADR 0009](adr/0009-semantic-evidence-in-scoring.md).

## Score and coverage

Each field also has an **evidence** fraction `e` in `[0, 1]`:
- 1 for a compared scalar, list or text;
- for an object, the shared leaves over the shared leaves plus the one-sided keys;
- 0 without similarity.

Over the fields `O` that give a similarity, with weights `w`:

```text
score    = Σ_{j ∈ O} w_j e_j s_j / Σ_{j ∈ O} w_j e_j
coverage = Σ_{j ∈ O} w_j e_j / Σ_j w_j
```

When no field gives evidence the score is absent, never 1. Coverage tells how much of the schema supported the score, so a 100 % coverage never hides missing information. It is lowered by a semantic field without embeddings, by a missing value, and by object keys on one side only: `{"port": 5432}` against `{"port": 5432, "tls": true}` scores 1 with a coverage of 0.5.

## Explanations

The comparison lists, for each field in schema order, its similarity, its signal, whether it contradicts, whether the contradiction vetoes, and whether evidence is one-sided. Exact matches additionally report the normalization steps they needed (see [duplicate definitions](duplicate-definitions.md)).

## Validation

- `crates/univo-core/tests/comparison_corpus.rs`:
  - every pair of the critical corpus respects the hard rules: no comparison across partitions; a veto exactly when a discriminant field contradicts; `compatible` pairs incomplete and not vetoed; `exact` pairs scored 1;
  - the score is symmetric and deterministic;
  - its AUC on the corpus (0.763 at U-008) is guarded against regressions.
- `crates/univo-engine/tests/semantic_corpus.rs`: with the default model's embeddings, the AUC is 0.776 (U-010), guarded at 0.756; it runs when the model's files are available, as in CI.
- `crates/univo-core/tests/semantic_comparison.rs`: cosine and clamping, no evidence without both embeddings, lexical and hybrid fields unaffected by embeddings.
- `crates/univo-core/tests/object_comparison.rs`: nested nulls are unknown, nested differences veto, one-sided keys lower the coverage, objects of unknown values give no evidence, empty objects are equal.
- `crates/univo-core/tests/lexical_properties.rs`: edit similarity and trigram Jaccard against naive reference implementations.
- Release budgets: one million comparisons of standard records in 550 ms (311 ms measured) and of configurations with nested parameters in 250 ms (143 ms measured).


U-011 search preserves verified whole-record equality when the pairwise score is absent because semantic tokens have no embedding. This is engine `SEARCH_VERSION=1`, independent of `SCORING_VERSION=2`; it assigns exact evidence only after the complete equality oracle succeeds, and does not turn partial unknown-token equality into semantic evidence. See [ADR 0010](adr/0010-python-finder-and-exact-fallback.md).
