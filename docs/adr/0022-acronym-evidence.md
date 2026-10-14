# 0022. Acronym evidence for text fields

- Status: Accepted
- Date: 2026-10-14
- Roadmap: U-012

## Context

Names are the weakest case of the quality benchmark.
- **The ROR validation split.** With the multilingual default model (ADR 0021), the semantic comparator recovers 32.9% of the validation aliases at ≥ 98% precision.
- **Acronyms.** 257 of the 1,348 validation aliases are acronyms written as the initials of the display name: "NIOSH" for "National Institute for Occupational Safety and Health". All of them were missed, with similarities near 0.1 lexically and semantically.

The benchmark could not measure the risk, so the analysis started there.
- **Protocol v1 had no traps.** Its negatives pair display names; none pairs an acronym with another organization.
- **Acronyms collide.** ROR itself lists the same acronym for several organizations: "SPF" names Sun Protection Foundation, South Plains Foundation and Swiss Paraplegic Foundation.

Protocol `ror-alias-v2` therefore adds a negative family, `shared_acronym`, built by `tools/build_quality_corpus.py`.
- **What it pairs.** Each acronym alias of a sampled organization meets the display names of up to two other **active organizations of the same split** that ROR lists with the same acronym, chosen in hash order across the whole dump.
- **Why it is independent of this rule.** It uses ROR's own acronym annotations, not the rule being evaluated.
- **Size.** It adds 189 validation and 207 test negatives. Every v1 pair is kept unchanged.

## Decision

- **Acronym rule.** A text is the acronym of another when both hold:
  - it is one word of 5 to 10 letters or digits, once dots, hyphens, ampersands and apostrophes are removed;
  - it equals, lowercased, the initials of the other text's words: of all of them, or of all except a fixed list of 46 articles, prepositions and conjunctions in the languages most common in organization names.

  The comparison ignores case, which snapshots do not keep, but not accents.
- **Similarity.** The field similarity is then at least 0.9, in every text mode: lexical, hybrid or semantic. Signal `acronym`.
- **Length bounds.**
  - **5 characters minimum.** Shorter acronyms give no evidence.
  - **No initials for long texts.** A text with 20 spaces or more is a description, not a name.
- **Cost.** The acronym and the two initials are hashed (32 bits each) when a record is prepared, and again when a snapshot is loaded, without allocation. A comparison compares hashes first and confirms on the texts, so a hash collision cannot create a match.
- **Candidates.** A per-text-field lane indexes these hashes by partition and role (acronym or initials). It is read when the minimum score is at most 0.9, so search keeps returning exactly the exhaustive oracle.
- **Versions.** `SCORING_VERSION` 3. Python's `Finder.config()` now reads the versions from the native module instead of repeating them.

## Alternatives

**Selection rule.** The U-012 calibration rule, on validation only: the highest recall at a threshold calibrated for ≥ 98% precision. Recall at 95% and 97% precision is shown to judge robustness.

| Variant (semantic field, protocol v2, validation) | AUC | Recall at P ≥ 95% | at 97% | at 98% |
| --- | --- | --- | --- | --- |
| No acronym evidence | 0.713 | 41.2% | 36.0% | 32.9% |
| Acronyms of ≥ 3 characters, 0.9 | 0.780 | 6.3% | 6.3% | 6.3% |
| Acronyms of ≥ 4 characters, 0.9 | 0.778 | 51.2% | 43.7% | 6.3% |
| **Acronyms of ≥ 5 characters, 0.9** | **0.734** | **44.4%** | **40.0%** | **36.4%** |
| Acronyms of ≥ 6 characters, 0.9 | 0.717 | 42.0% | 36.7% | 33.7% |

Acronym pairs of the validation split, by acronym length:

| Characters | Duplicates | Distinct organizations | Share distinct |
| --- | --- | --- | --- |
| 2 | 7 | 7 | 50% |
| 3 | 112 | 96 | 46% |
| 4 | 99 | 8 | 7% |
| 5 | 32 | 0 | 0% |
| 6 to 10 | 10 | 0 | 0% |

- **Three or four characters.** These add far more recall at 95% precision, but a two- or three-letter acronym is a coin toss on the name alone. A calibration at 98% must then place the threshold above 0.9, and recall collapses to 6%. They need another field, such as a country or a partition, which this name-only benchmark cannot evaluate.
- **A similarity graded by length**, (L−1)/L for L ≥ 4. It is no better than the cutoff at 95–97%, and gives 26.9% at 98%.
- **Prefixes of words**, which allow "APsaA" for "American Psychoanalytic Association". About one more point of recall at 98%, but its keys cannot be enumerated cheaply for an index.
- **Removing a one-sided parenthetical qualifier**, as in "Baidu (China)" against "Baidu". It lowered recall at 98% (27.5%): on the name alone, "MSD" against "MSD (Spain)" is indistinguishable, and ROR labels these as distinct organizations.
- **Using case to detect acronyms.** Snapshots keep only case-folded text, and the five-character minimum makes coincidental matches between ordinary words and initials negligible: 0 among the validation's distinct pairs.

## Evidence

- **Setup:** Apple M2, macOS 27.0.1, CPython 3.14.7. Release wheels installed outside the source tree, run with Rust absent from PATH. Protocol `ror-alias-v2`; the test split was evaluated after the rule was fixed on validation.
- **Bug found by mutation tests.** The first v2 build had a deduplication bug that dropped some shared-acronym negatives (188 validation, 202 test). The test split was also scored on that build, with the same rule. The rule was not changed, and after the fix it gives the same validation selection and the same test figures.

| Comparator | Validation recall at P ≥ 98% | Test recall | Test precision | False matches (of 1,380) | Shared-acronym false matches (of 207) | Test AUC |
| --- | --- | --- | --- | --- | --- | --- |
| Lexical, scoring 2 | 4.9% | 5.6% | 93.8% | 5 | 0 | 0.438 |
| **Lexical, scoring 3** | 10.2% | **10.7%** | 94.1% | 9 | 1 | 0.468 |
| Default model, scoring 2 | 32.9% | 30.9% | 95.6% | 19 | 0 | 0.704 |
| **Default model, scoring 3** | 36.4% | **35.2%** | 95.7% | 21 | 1 | 0.726 |

- **Critical corpus:** every hard rule still passes, and the AUC is unchanged: 0.763 for lexical scoring and 0.777 with embeddings in the Rust tests, 0.785 for full profiles.
- **Speed and memory, release builds:**
  - **Comparison:** one million standard record pairs take 333 ns per pair against 327 on `main`; configuration pairs take 158 ns against 157.
  - **Preparation:** 1,290 ns per record against 1,140 (+13%), within the 1,500 ns budget.
  - **End to end at 100,000 records, every core:** lexical all-pairs takes 26.1 s against 26.4 s on `main`, and semantic all-pairs 8.15 s against 8.17 s. Removals and peak memory are the same.

## Consequences

- **Snapshots:** those saved with scoring version 2 are refused, as for any scoring change, and must be rebuilt from the records. Embedding caches are unaffected.
- **Precision:** there are more false matches between distinct organizations that share an acronym. On test this was one of 207 shared-acronym traps; on real data it depends on how many organizations share five-letter acronyms.
- **Quality gates:** they still fail on names (98% precision and 90% recall). This changes no threshold and calibrates no profile.
- **Memory:** each prepared text field keeps 12 bytes of hashes. Each text field index holds posting lists only for texts with an acronym or initials of 5 to 10 characters.

## Revisit when

- **Multi-field benchmark.** A benchmark with a country, city or partition field can evaluate three- and four-letter acronyms combined with that field.
- **Words beyond initials.** Users report acronyms built from word prefixes ("APsaA") or with digits that this rule misses.
- **Real data.** Production data shows shared five-letter acronyms often enough to matter.
