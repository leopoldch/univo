# 0021. Multilingual default model

- Status: Accepted
- Date: 2026-10-13
- Roadmap: U-004 follow-up, U-012
- Supersedes: the default-model choice of [ADR 0007](0007-default-embedding-model.md). Its backend and model-selection contract are unchanged.

## Context

ADR 0007 proposed `minishlab/potion-base-8M`, which is English only: an English WordPiece vocabulary, with lowercasing and accent stripping. The maintainer requires a default model that understands every language. Two constraints apply:

- [specs/07-performance.md](../../specs/07-performance.md) caps the default download at 100 MiB.
- [specs/04-models-and-cache.md](../../specs/04-models-and-cache.md) forbids announcing a multilingual model unless languages are measured separately.

The only multilingual Model2Vec model, `minishlab/potion-multilingual-128M`, weighs 512 MB, and the U-004 spike found it no better than the English models.

## Decision

- **Default model:** [`sentence-transformers/static-similarity-mrl-multilingual-v1`](https://huggingface.co/sentence-transformers/static-similarity-mrl-multilingual-v1) at commit `b68f4122`, used without fine-tuning.
  - License Apache-2.0; 50 languages; multilingual BERT vocabulary, uncased.
  - The model is static: no network runs at inference.
  - Training used Matryoshka representation learning, so the leading dimensions carry the most information.
- **Reduced artifact:**
  - int8 weights following Model2Vec's recipe, keeping the first 128 of 1,024 dimensions, in the Model2Vec layout.
  - Three files totalling 15.3 MB: `config.json` (79 B), `model.safetensors` (13.6 MB) and `tokenizer.json` (1.7 MB).
  - [`tools/build_default_model.py`](../../tools/build_default_model.py) rebuilds the files byte for byte from the pinned upstream files, with the Python standard library only, and checks the checksums that `registry::default_model` compiles into the release.
- **Hosting:** the project publishes the files, with a model card and the Apache-2.0 license, at `univo-models/static-similarity-mrl-multilingual-int8-128` on Hugging Face.
  - The `univo` namespace is taken by an unrelated account ([ADR 0001](0001-project-name-and-license.md)).
  - The release pins the commit of that repository. The maintainer publishes it; nothing is uploaded automatically.
- **Language claim:** the default is announced as multilingual.
  - **Measured gains:** overall recall on validation, 32.9% against 19.6% at the same ≥ 98% precision. The exploratory test split shows the gain mostly on cross-language name pairs, and also in English. Independent confirmation is pending.
  - **Equal:** French on the critical corpus.
  - **Not claimed:** French or other single languages on their own, where samples are too small.

## Alternatives

**Selection rule.** The U-012 calibration rule picks the variant:
- calibrate a threshold on the validation split, at the highest recall with precision ≥ 98% and at least 20 true positives;
- keep the candidate with the highest validation recall at that threshold;
- on a tie, keep the smaller files.

The critical corpus is a regression set: it vetoes a candidate that breaks a rule, but it ranks none. Validation split: 1,348 alias pairs and 1,173 distinct pairs.

| Candidate | Files | Validation recall | Validation precision | False matches | Critical text AUC |
| --- | --- | --- | --- | --- | --- |
| potion-base-8M (ADR 0007) | 30.9 MB | 19.6% | 98.1% | 5 | 0.595 |
| potion-multilingual-128M, int8, 128 dims | 82.7 MB | 18.3% | 98.0% | 5 | 0.596 |
| static-similarity-mrl, float32, 1,024 dims | 435.4 MB | 30.9% | 98.1% | 8 | 0.655 |
| static-similarity-mrl, int8, 256 dims | 28.8 MB | 30.0% | 98.1% | 8 | 0.634 |
| **static-similarity-mrl, int8, 128 dims** | **15.3 MB** | **32.9%** | **98.0%** | **9** | **0.658** |
| static-similarity-mrl, int8, 64 dims | 8.5 MB | 29.3% | 98.0% | 8 | 0.645 |
| Lexical only | 0 | 4.9% | 98.5% | 1 | 0.633 |

**Test split.** The spike also printed the held-out test results of every variant before this rule was written down. The rule picks the same variant on validation alone. Even so, the test split no longer evaluates this choice independently: its figures below are exploratory. Confirming the choice needs a held-out set nobody has seen (see *Revisit when*).

- **Keep potion-base-8M.** It has the fewest false matches, but it is English only and recovers 40% fewer aliases on validation.
- **potion-multilingual-128M reduced.** It is larger, it peaks at 1.2 GiB while loading, and it is not better than the English model here.
- **static-similarity-mrl at 256 dimensions.** On validation it finds 40 fewer aliases (404 against 444) for one fewer false match (8 against 9), at twice the size and twice the index memory. Prefer it only if a reviewed benchmark shows that false matches matter more than recall.
- **64 dimensions.** It is smaller still, but lower on validation recall and on the critical corpus.
- **Fine-tuning first.** Not done:
  - The weakness that remains is "same topic, other entity" and acronyms. Lexical, structured and veto evidence handle those better than a model.
  - Fine-tuning would also add a training protocol and data that the project does not have yet. Reconsider it with the reviewed labels of U-012.
- **Contextual encoders.** Rejected again for the reasons of ADR 0007: they are 100 to 500 times slower and weigh hundreds of MB, with a runtime in the wheel.

## Evidence

- **Machine and builds:** Apple M2, macOS 27.0.1, release builds.
- **Quality:** from the [U-012 benchmark](../benchmarks.md#name-quality), on ROR name aliases with a validation-chosen threshold.
  - The test split has 1,341 duplicate and 1,173 distinct pairs. It is exploratory for this choice (see *Alternatives*).
  - A release wheel installed outside the source tree, with Rust absent from PATH, reproduced the spike figures exactly.
- **Speed:** from `univo-embed`'s budget tests and the engine's search budget.

| Measure | potion-base-8M | New default |
| --- | --- | --- |
| Exploratory test recall, all / English / cross-language | 17.9% / 44.0% / 8.3% | 30.9% / 54.6% / 48.8% |
| Exploratory test precision (false matches out of 1,173) | 96.8% (8) | 95.6% (19) |
| French pairs (32 duplicates) | 2 found | 3 found |
| Full critical profile AUC, all / EN / FR | 0.784 / 0.831 / 0.708 | 0.785 / 0.830 / 0.705 |
| Critical corpus AUC with embeddings (`semantic_corpus` test) | 0.776 | 0.777 |
| Exploratory test AUC, unreturned scores ranked last | 0.747 | 0.690 |
| Load and first encoding, median of 5 | 18 ms | 60 to 95 ms |
| 20,000 descriptions, 1 / 8 threads | 0.95 s / — | 0.82 s / 0.27 s |
| Exhaustive top-10 over 100,000 vectors | 2.5 ms | 1.2 ms |
| End-to-end search p95 at 100,000 records, one partition | 10.0 ms | 8.5 ms |
| Index memory at 100,000 records (search budget workload) | 156 MiB | 92 MiB |

On the exploratory test split, recall at the operating threshold is 1.7 times higher, for 11 more false matches out of 1,173 distinct pairs. The test AUC is lower because more pairs fall below a zero cosine and count as unreturned.

## Consequences

- **Compatibility:**
  - Vectors of the two models are not comparable, and model identity includes repository, commit and file digests.
  - Embedding caches built with potion-base-8M are not reused.
  - A snapshot saved with the previous default keeps its resolved model and still loads it from the hub. It is never mixed with the new one.
- **Memory and speed:** index vectors take half the memory, and dense scans are twice as fast. Loading takes 60 to 95 ms instead of 18 ms, still far below the 2 s budget, because the vocabulary has 105,879 tokens.
- **Language behavior:** the tokenizer lowercases and strips accents like the previous one, and covers non-Latin scripts.
- **Precision:** there are more false matches between distinct organizations with similar names (19 against 8 out of 1,173). The quality gates still fail on names, as before; this choice does not calibrate a profile (U-012).
- **License and maintenance:**
  - The model is Apache-2.0 and is downloaded at run time, not shipped in wheels.
  - The published repository carries the license, the attribution and a notice of the modifications.
  - The project maintains that repository. A change to its files needs a new release and new checksums.
- **Test setup:** tests that use the real model rebuild it with the tool, or download it from the published repository.

## Revisit when

- **The independent confirmation is still pending.** Build a held-out set from ROR entities absent from v2.14 (a later dump), with the same builder and the same disjointness rules. Evaluate it once, with this model and potion-base-8M, and record the result here before claiming the gain as validated.

- U-012's reviewed labels show that precision on distinct entities matters more than recall: compare the 256-dimension variant.
- A fine-tuned or distilled static model beats this one on the same held-out split.
- Model2Vec or Sentence Transformers publishes a multilingual static model with better measured results under the 100 MiB budget.
