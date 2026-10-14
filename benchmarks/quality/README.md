# U-012: offline name calibration

The benchmark and calibration tooling are implemented. The quality gates fail on this task; no production threshold or alpha publication follows from these results. The default model changed afterwards on this benchmark's evidence ([ADR 0021](../../docs/adr/0021-multilingual-default-model.md)). [ADR 0011](../../docs/adr/0011-offline-quality-calibration.md) records the decision.

## Reproduce

Download [ROR v2.14](https://zenodo.org/records/23187250), then run the builder with the versioned ZIP. The downloaded archive matched the publisher’s MD5; its SHA256 is pinned in the builder; [corpus.json](corpus.json) pins the generated pair SHA256. The data is not included in the wheel or repository.

```bash
python tools/build_quality_corpus.py /path/to/v2.14-2026-10-06-ror-data.zip --output /tmp/ror-pairs.jsonl
python tools/evaluate_quality.py /tmp/ror-pairs.jsonl \
  --critical tests/fixtures/critical/cases.jsonl --output /tmp/lexical.json
python tools/evaluate_quality.py /tmp/ror-pairs.jsonl --mode semantic \
  --model-path /path/to/local/model --critical tests/fixtures/critical/cases.jsonl \
  --output /tmp/semantic.json
```

Use a release wheel installed outside the source tree. Run each model in a fresh process, with model files already present. The evaluator always uses `offline=True`, one thread, no model API and no third-party Python library. Rust performs normalization, encoding, scoring and candidate retrieval. Python computes benchmark metrics, not record similarities. The builder and metrics tests also run without an installed wheel.

## Corpus and protocol

[ROR](https://ror.readme.io/docs/data-dump) provides names and organization identifiers under CC0. The archive also includes GeoNames location data under CC BY 4.0; the builder exports no locations. The versioned [Zenodo release](https://zenodo.org/records/23187250) states both terms. Only active ROR entities with at least one alternate name are eligible. Hash ordering selects 4,000 entities without inspecting model scores.

Each entity's display name is paired with up to three source aliases/labels/acronyms, sorted alphabetically. No typo generation supplies easy positives. Negatives pair distinct ROR IDs within the same split: adjacent display names in alphabetical order (a hard-negative proxy), plus an offset halfway through the list. Reversed negative pairs are deduplicated. A distinct ROR ID is the benchmark identity rule; this is not a manual annotation of production duplicates. Departments, renamed institutions and homonyms can be ambiguous. A double-reviewed ambiguous subset remains necessary before a quality claim.

SHA256 of each ROR ID chooses train (60%), validation (20%) or test (20%). Both ends of every negative come from that split. All aliases of an entity stay together. Train is reserved and unused; no weights or model are fitted. Validation picks the threshold with maximum empirical recall at precision ≥98% and at least 20 true positives. Tied scores are processed together; higher thresholds break recall ties. If no threshold qualifies, calibration abstains. There is no fallback selected from test results.

| Split | Entities | Positive pairs | Negative pairs |
| --- | --- | --- | --- |
| Train, reserved | 2,436 | 4,283 | 3,654 |
| Validation | 782 | 1,348 | 1,173 |
| Held-out test | 782 | 1,341 | 1,173 |

The 12,972 pairs are a name-alias benchmark, not a representative deployment distribution. Negatives are sampled; results do not measure every pair in the registry. Language metadata comes from ROR; absent language tags are `unknown`, not guessed. Precision intervals are Wilson intervals treating pairs independently; shared entities make them descriptive rather than entity-cluster confidence guarantees.

## Results

2026-10-11, Apple M2 (8 cores), 16 GiB, macOS 27.0.1, CPython 3.14.7, Rust 1.99.0. Portable CPU release macOS arm64 wheel from merged U-011, fresh measuring process per model. OS file cache was not purged. Model footprint is the sum of the three required local files, not an inferred hub download size. Peak RSS includes inputs, native model/index allocation and sequential evaluation stages; it is not idle engine RSS. Elapsed tool time is not an inference latency benchmark.

| Name comparator | Validation threshold | Test precision | Test recall | Test AUC | Model files | Process peak RSS |
| --- | --- | --- | --- | --- | --- | --- |
| [Lexical](lexical.json) | 0.80000001 | 93.75% | 5.59% | 0.401 | 0 MiB | 81.7 MiB |
| [potion-base-2M](potion-base-2M.json) | 0.76844931 | 95.95% | 17.67% | 0.725 | 7.86 MiB | 96.6 MiB |
| [potion-base-8M](potion-base-8M.json) | 0.73860359 | 96.77% | 17.90% | 0.747 | 29.49 MiB | 142.3 MiB |
| [potion-multilingual-128M](potion-multilingual-128M.json) | 0.77333266 | 96.02% | 16.18% | 0.705 | 506.38 MiB | 2,045.1 MiB |
| [static-similarity-mrl, int8, 128 dims](static-similarity-mrl-multilingual-int8-128.json), default since ADR 0021 | 0.66665548 | 95.62% | 30.95% | 0.690 | 14.56 MiB | 173.7 MiB |

The default model's row was measured on 2026-10-13 with the same protocol: the same machine and CPython, a release wheel from the ADR 0021 branch installed outside the tree, and Rust absent from PATH. Its gain comes mostly from cross-language aliases, a display name and an alias tagged with different languages: 177 of 363 found, against 30 for potion-base-8M. In English it finds 159 of 291, against 128. It also accepts 19 distinct pairs, against 8. Its test AUC is lower because more pairs score at or below zero and count as unreturned.

These test figures are exploratory for the model choice. The spike printed test results for several variants (64 to 1,024 dimensions) before ADR 0021 fixed the selection rule. The rule picks the 128-dimension variant on validation alone: 32.9% validation recall, against 30.9% at 1,024 dimensions, 30.0% at 256 and 29.3% at 64. An unseen held-out set must still confirm the gain.

No comparator meets the proposed 98% precision / 90% recall gates on test. Exact normalized names return one true and two false positives: identical names are not proof of organization identity. This baseline has tiny support and demonstrates homonyms, not a universal exact-match precision estimate.

Scores come from exhaustive `search`, with output limit one because each query has exactly one eligible target in its pair partition. Scores not returned at `1e-12` remain in the denominator, below all returned scores for AUC; this includes zero scores and absent evidence. They are not discarded as an easier subset.

Candidate generation is measured separately at the validation threshold, by checking `Report.candidates` before final scoring accepts a match. On this one-field task it equals final recall. Every model has zero oracle disagreements and zero query truncations. This proves agreement for the sampled isolated pairs, not full-registry candidate recall, ANN recall or the 99% candidate gate in a large retrieval workload. Scoring profiles and ANN are unchanged.

## Protocol v2: shared acronyms

Protocol `ror-alias-v2` ([corpus](ror-alias-v2/corpus.json)) keeps every v1 pair and adds one negative family, `shared_acronym`.
- **What it pairs.** Each acronym alias of a sampled organization meets the display names of up to two other active organizations of the same split that ROR lists with the same acronym, chosen in hash order across the whole dump.
- **Why.** "SPF" against "South Plains Foundation", when "SPF" is the alias of Sun Protection Foundation. v1 had no such pair, so it could not measure the risk of acronym evidence.
- **Size.** 189 validation and 207 test pairs (886 train, reserved).

The builder writes v2; rebuild with the command above. Results with scoring version 3, which adds acronym evidence ([ADR 0022](../../docs/adr/0022-acronym-evidence.md)), against version 2. Same machine and protocol as above, on 2026-10-14:

| Comparator | Validation threshold | Test precision | Test recall | Test AUC | Shared-acronym false matches |
| --- | --- | --- | --- | --- | --- |
| [Lexical, scoring 2](ror-alias-v2/lexical-scoring-2.json) | 0.80000001 | 93.75% | 5.59% | 0.438 | 0 of 207 |
| [Lexical, scoring 3](ror-alias-v2/lexical.json) | 0.74074078 | 94.12% | 10.74% | 0.468 | 1 of 207 |
| [Default model, scoring 2](ror-alias-v2/default-model-scoring-2.json) | 0.66665548 | 95.62% | 30.95% | 0.704 | 0 of 207 |
| [Default model, scoring 3](ror-alias-v2/default-model.json) | 0.66106427 | 95.74% | 35.20% | 0.726 | 1 of 207 |

- **Selection on validation only.** The acronym rule (five characters or more, similarity 0.9) was chosen there; ADR 0022 lists the alternatives.
- **The test split was scored once with that rule.** That was on a first v2 build, whose deduplication bug dropped some shared-acronym negatives. Mutation tests found the bug. The fixed corpus gives the same selection and the figures above.
- **The quality gates still fail.**

## Critical regression and languages

The 234 critical cases never choose a threshold. Full profile evaluation checks all veto, partition, invalid-input and exact-kind rules through Finder; all pass. AUC uses all 178 unconstrained duplicate/distinct pairs, including unavailable scores as last-ranked ties. Earlier reports excluded some unavailable scores, so their denominator differs. Plain strings/names remain lexical in these profiles; descriptions are lexical for the lexical baseline and semantic for the model runs.

| Full profile score | All, 178 pairs | EN, 117 pairs | FR, 49 pairs |
| --- | --- | --- | --- |
| Lexical | 0.755 | 0.798 | 0.683 |
| potion-base-2M | 0.786 | 0.831 | 0.718 |
| potion-base-8M | 0.784 | 0.831 | 0.708 |
| potion-multilingual-128M | 0.786 | 0.829 | 0.716 |
| static-similarity-mrl, int8, 128 dims | 0.785 | 0.830 | 0.705 |

The raw files also include pure lexical/semantic text-view AUC per language, family and view (234 views), plus test metrics per ROR language/family. Other critical languages have at most four record pairs; some have only one class, so AUC is null. Three cross-language record pairs cannot establish cross-language quality. The French difference does not establish a multilingual advantage and does not justify exceeding the 100 MiB default-model budget.

SemHash and a full 100k semantic Finder performance workload remain unmeasured in U-012; the earlier [U-004 model spike](../spikes/u004/README.md) and [U-011 lexical measurements](../../docs/performance.md) are separate protocols. Supervised Dedupe/Splink comparisons are not applicable to this unsupervised name-only setup without introducing a separately declared training protocol. Alpha publication and the release quality decision remain pending.

## Validation

The installed release wheel passes 1,071 Python/tool/corpus tests (three conditional skips) outside the repository with Rust absent from PATH, on CPython 3.12 and 3.14. Formatting, ruff, mypy and documentation links pass. No Rust implementation or wheel contents change in this task, so the existing native suites and wheel-size measurements remain U-011 results.
