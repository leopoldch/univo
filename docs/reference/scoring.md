# Pairwise scoring

How two prepared records are compared, field by field, into a score, a coverage and a list of contradictions. This implements [specs/03-matching.md §10](../../specs/03-matching.md) with the lexical signals of U-008 and the semantic similarity of U-010 ([ADR 0009](../adr/0009-semantic-evidence-in-scoring.md)). The rules are versioned by `SCORING_VERSION`, now 4: any change in a similarity, a signal or a score increments it. Version 3 adds acronym evidence ([ADR 0022](../adr/0022-acronym-evidence.md)), version 4 distinct words ([ADR 0023](../adr/0023-distinct-words.md)); snapshots saved with an earlier version are refused and must be rebuilt from the records.

Scores rank pairs. **They are not probabilities**, and no threshold is defined here: thresholds are calibrated on a separate dataset (U-012).

## Comparing two records

`compare(schema, left, right)` takes two records prepared with the same schema; `compare_with_vectors` also takes the embeddings of their semantic fields, as the engine computes them (see [vector search](../design/vector-search.md)). Without embeddings, semantic fields give no evidence.

- Records of different partitions are never compared: the result is `None`.
- Records prepared with another schema are refused with `state.schema_mismatch`.

Otherwise each field yields a similarity in `[0, 1]`, or no similarity when it gives no evidence.

| Field | Similarity | Signal |
| --- | --- | --- |
| absent, null or blank on at least one side | none; `incomplete` when the other side has a value | `missing` |
| text, lexical mode | the larger of the edit similarity and the trigram similarity | `edit_distance` or `trigrams` |
| text, semantic mode | the cosine of the two embeddings, clamped to `[0, 1]`; none without both embeddings | `semantic` |
| text, hybrid mode | as lexical mode; hybrid fields are not embedded until a combination is validated (U-012) | `edit_distance` or `trigrams` |
| text, any mode, one side the acronym of the other's words | at least 0.9: the larger of 0.9 and the mode's similarity | `acronym` when 0.9 is larger |
| text, any mode, names with distinct words | 0.8 × the mode's similarity, with a contradiction (never a veto) | the mode's signal |
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

`lexical_similarity` is this rule, shared with the lexical index ([lexical candidates](../design/lexical-search.md)), so that candidates and scores never disagree.
  - Trigrams are extracted once, when a record is prepared, and kept sorted, so a comparison is a linear merge.
  - Only lexical and hybrid text fields keep them, which is why the text mode belongs to the schema's preparation fingerprint.

## Acronym signal

A text is the acronym of another when it is one word of 5 to 10 letters or digits, once dots, hyphens, ampersands and apostrophes are removed, and it equals the initials of the other text's words, lowercased. The initials are taken of every word, or of every word except a short list of articles, prepositions and conjunctions ("of", "the", "and", "de", "für", …). "N.I.O.S.H." is the acronym of "National Institute for Occupational Safety and Health".

- **Similarity 0.9, below an exact match.** An acronym is strong evidence, not identity: on the ROR benchmark, acronyms of five letters or more matched another organization's name in none of the validation's hard negatives, but four letters did in 7% of cases and three in about half.
- **Shorter acronyms give no evidence.** "IOR" or "BLM" name too many organizations; a partition or another field must decide.
- **Names, not descriptions.** A text with 20 spaces or more has no initials.
- **Cost.** The acronym and the initials are hashed once, when the record is prepared, and when a snapshot is loaded. A comparison first compares these hashes, then confirms on the texts.

The rule applies to lexical, hybrid and semantic fields alike, and the [acronym lane](../design/search.md) finds these pairs whatever their lexical or semantic similarity. The measurements are in [ADR 0022](../adr/0022-acronym-evidence.md).

## Distinct words

Two names built on one template, each with a word of its own, such as "National Institute of Epidemiology" and "National Institute of Oncology", get a high lexical or semantic similarity, yet name different organizations. They have **distinct words** when:
- **they share a content word**: a word other than the articles, prepositions and conjunctions of the acronym rule;
- **each has a content word of its own**: one related to no content word of the other.

Two words are related when they are equal, when one begins the other and has 3 characters or more ("univ" and "university"), or when their edit similarity is 0.8 or more ("universitat" and "universität").

- **Effect.** The field similarity is multiplied by 0.8, and the field reports a contradiction, which a text field never turns into a veto. When acronym evidence wins, there is no contradiction.
- **Not for one-sided words.** "Instituto Federal Goiano" against "Instituto Federal de Educação, Ciência e Tecnologia Goiano" is not penalized: only one side has words of its own.
- **Names, not descriptions.** As for acronyms, a text with 20 spaces or more never has distinct words, nor does a text with more than 16 content words, whatever separates them.
- **Cost.** A 64-bit summary of each text's content words is computed when the record is prepared and when a snapshot is loaded. A comparison looks at the words only when the two summaries share a bit.

The rule lowers similarities and never raises one, so candidate search stays complete. The measurements are in [ADR 0023](../adr/0023-distinct-words.md).

## Semantic signal

A semantic field compares the unit embeddings of its two normalized texts, from the model chosen for the index ([models](../guide/models.md)). The similarity is their cosine clamped to `[0, 1]`: opposite and unrelated texts are equally dissimilar. A text without any token the model knows has no embedding, and the field then gives no evidence rather than a made-up similarity.

Hybrid fields are scored lexically. On the critical corpus, adding their cosine to the score, as the larger or the mean of both signals, lowered the ranking quality; the measurements are in [ADR 0009](../adr/0009-semantic-evidence-in-scoring.md).

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
  - its AUC on the corpus (0.763 at U-008, 0.795 with distinct words) is guarded against regressions.
- `crates/univo-engine/tests/semantic_corpus.rs`: with the default model's embeddings, the AUC is 0.776 (U-010) and 0.827 with distinct words, guarded at 0.756; it runs when the model's files are available, as in CI.
- `crates/univo-core/tests/semantic_comparison.rs`: cosine and clamping, no evidence without both embeddings, lexical and hybrid fields unaffected by embeddings.
- `crates/univo-core/tests/object_comparison.rs`: nested nulls are unknown, nested differences veto, one-sided keys lower the coverage, objects of unknown values give no evidence, empty objects are equal.
- `crates/univo-core/tests/lexical_properties.rs`: edit similarity and trigram Jaccard against naive reference implementations.
- `crates/univo-core/src/acronym.rs`: acronyms and initials, skipped words, case and accents, the 20-space limit, and prepared hashes that select exactly the pairs the text rule accepts.
- Release budgets: one million comparisons of standard records in 550 ms (311 ms measured at U-008, 333 ms with acronym evidence) and of configurations with nested parameters in 250 ms (143 ms, then 158 ms).


Search preserves verified whole-record equality when the pairwise score is absent because semantic tokens have no embedding. This is engine `SEARCH_VERSION=1`, independent of `SCORING_VERSION=2`; it assigns exact evidence only after the complete equality oracle succeeds, and does not turn partial unknown-token equality into semantic evidence. See [ADR 0010](../adr/0010-python-finder-and-exact-fallback.md).
