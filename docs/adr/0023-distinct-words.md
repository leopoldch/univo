# 0023. Distinct words lower the similarity of names

- Status: Accepted
- Date: 2026-10-14
- Roadmap: U-012

## Context

After acronym evidence (ADR 0022), the default model recovers 36.4% of the ROR validation aliases at ≥ 98% precision. The false matches near the calibrated threshold are names built on one template, each with a word of its own:

| Similarity | Pair (distinct organizations) |
| --- | --- |
| 0.821 | École Supérieure **de Gestion** / École Supérieure **des Affaires** |
| 0.809 | National Institute of **Epidemiology** / National Institute of **Oncology** |
| 0.774 | Health **Research Authority** / Health **Sciences Research Foundation** |

They force the threshold up. The validation curve shows the cost: recall goes from 36.6% to 45.3% between 0.66 and 0.58, while false matches go from 11 to 35.

## Decision

- **Distinct words.** Two names have distinct words when they share a content word and each has a content word related to no content word of the other.
  - **Content words** are all words except the 46 articles, prepositions and conjunctions of the acronym rule.
  - **Related words** are equal, or one begins the other and has at least 3 characters, or their edit similarity is at least 0.8.
- **Effect.** The field similarity, lexical or semantic, is multiplied by **0.8**.
  - **Contradiction.** The field reports a contradiction. A text field never turns it into a veto, and acronym evidence, when it wins, outweighs it.
  - **Names only.** Texts with 20 spaces or more are descriptions, as for acronyms, and never have distinct words.
- **Cost.** Each prepared text keeps a 64-bit summary of its content words, recomputed when a snapshot is loaded; the snapshot format is unchanged. A comparison reads the words only when the two summaries share a bit, and then decides on the texts. Variants are tested by a bounded edit distance, equivalent to the 0.8 threshold of `edit_similarity`.
- **Candidates.** The rule only lowers similarities, so every lane still proposes a superset of the matches, and search keeps returning exactly the exhaustive oracle.
- **Versions.** `SCORING_VERSION` 4. Snapshots of version 3 are refused and must be rebuilt from their records.

## Alternatives

**Selection.** The U-012 rule, on validation only: the highest recall at a threshold calibrated for ≥ 98% precision. The factor was chosen on a grid. The result was then checked on the two entity-disjoint halves of validation, which were not used to choose the rule.

| Factor (protocol v2, validation) | Default model | Half A | Half B | Lexical |
| --- | --- | --- | --- | --- |
| 1.0 (scoring 3) | 36.4% | 36.4% | 36.2% | 10.2% |
| 0.95 | 38.9% | 38.2% | 40.0% | 11.1% |
| 0.9 | 41.9% | 40.8% | 42.8% | 12.6% |
| 0.85 | 43.8% | 43.1% | 44.0% | 13.6% |
| **0.8** | **43.0%** | **42.7%** | **43.4%** | **14.3%** |
| 0.75 | 44.1% | 44.8% | 43.1% | 15.3% |
| 0.7 | 44.4% | 44.9% | 44.4% | 15.4% |

- **The factor.** The gain plateaus from 0.85 to 0.7. The value 0.8 is the middle of that plateau. A stronger factor gains under a point on validation and caps more true aliases with words of their own: "Grimsby Institute" and "Grimsby College" are one organization in ROR.
- **Partial acronyms**, such as "Jade UAS" for "Jade University of Applied Sciences", anchored on a shared word.
  - **What they gave.** 20 more validation aliases with no false match, about 1.5 points.
  - **Why not now.** They need a new candidate lane whose keys grow with the square of the number of words, and are left for a later block.
- **A penalty on lexical similarity alone.** Embeddings conflate templates at least as much as trigrams, and the rule gains on both comparators.

## Evidence

- **Setup.** Apple M2, macOS 27.0.1, CPython 3.14.7. Release wheels installed outside the source tree. Protocol `ror-alias-v2`.
- **Test split.** It was evaluated once, after the factor was fixed on validation.

| Comparator | Validation recall at P ≥ 98% | Test recall | Test precision | False matches (of 1,380) | Shared-acronym false matches (of 207) | Test AUC |
| --- | --- | --- | --- | --- | --- | --- |
| Lexical, scoring 3 | 10.2% | 10.7% | 94.1% | 9 | 1 | 0.468 |
| **Lexical, scoring 4** | 14.3% | **14.5%** | 95.6% | 9 | 1 | 0.461 |
| Default model, scoring 3 | 36.4% | 35.2% | 95.7% | 21 | 1 | 0.726 |
| **Default model, scoring 4** | 43.0% | **40.8%** | 96.0% | 23 | 2 | 0.730 |

- **Where the gain comes from.** The default model finds 75 more test aliases (472 to 547), mostly across languages (180 to 242), as the calibrated threshold drops from 0.661 to 0.557. English aliases go from 169 to 160.
- **Critical corpus.** It is independent of ROR and was not used to choose the rule.
  - **Pairwise AUC without embeddings:** 0.763 to 0.795.
  - **Pairwise AUC with the default model:** 0.777 to 0.827.
  - **Full profiles:** lexical 0.755 to 0.815, default model 0.785 to 0.833.
  - **Rules.** Every veto, partition and invalid-input rule still passes.
- **100,000 real ROR names**, all pairs at 0.8. Names of the same organization are counted from ROR's identifiers.

  | | Matches | Of which within one organization |
  | --- | --- | --- |
  | Lexical, scoring 3 | 85,063 | 2,690 (3.2%) |
  | Lexical, scoring 4 | 8,276 | 2,334 (28.2%) |
  | Default model, scoring 3 | 123,940 | 6,897 (5.6%) |
  | Default model, scoring 4 | 67,187 | 5,124 (7.6%) |

  Of the pairs no longer matched at 0.8, 99.5% (lexical) and 97% (semantic) join different organizations.
- **Cost, same machine, release.**
  - **Comparison budget, 1,000,000 pairs of standard records.** 325 ns to 371 ns per pair. Its synthetic names share a word in 14% of pairs, from a 23-word vocabulary.
  - **Preparation.** 1,175 ns to 1,255 ns per record.
  - **All-pairs `find` at 100,000 names, through the wheel, ABBA order.**
    - lexical on ROR names: 2.06–2.09 s to 2.15–2.17 s;
    - semantic on synthetic names: 4.49–4.56 s to 4.46–4.52 s;
    - lexical on synthetic names: 1.91–1.97 s to 1.91–1.93 s.

## Consequences

- **Scores.** They change for names with distinct words: version 4, and snapshots must be rebuilt.
- **Fixed thresholds.** A threshold fixed under version 3 now matches fewer template names. Thresholds calibrated on validation drop.
- **Explanations.** Reports list such fields among their conflicts.
- **Gates.** The quality gates of U-012 still fail. Short acronyms need another field, and translations and names without a shared word stay model-limited.

## Revisit when

- **A multi-field benchmark exists.** With a country, for instance, the factor may be relaxed when another field confirms identity.
- **Renamed or rebranded organizations** with one-word differences ("Randolph Community College" / "Randolph Technical College") become a priority.
- **A model distinguishes templates itself.** The rule then becomes redundant.
