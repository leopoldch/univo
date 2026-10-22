# Benchmarks

Univo against [SemHash](https://github.com/MinishLab/semhash) 0.5.0, the closest existing tool: local deduplication with the same family of static models. Both tools run offline with the same model files. Raw results and scripts are in [benchmarks/](../benchmarks).

**Machine.** Apple M2 (4 performance and 4 efficiency cores), 16 GiB, macOS 27.0, CPython 3.14.7 for Univo and 3.12.14 for SemHash, release wheels installed outside the source tree. Timings are from Univo 1.0.1 as published on PyPI, on mains power, with each tool run in alternation (A B B A).

## Speed

[scale.py](../benchmarks/comparison/scale.py) deduplicates synthetic company names in one partition. One record in ten repeats an earlier name with a typo. Each tool runs at its default threshold, on every core, and turns pairs into removals with SemHash's rule inside the timed section. Wall time includes importing the tool and loading the model.

| Records | Univo, semantic | SemHash |
| --- | --- | --- |
| 10,000 | **0.17–0.19 s**, 150 MiB | 0.79–1.05 s, 156 MiB |
| 30,000 | **0.58–0.59 s**, 175 MiB | 1.90–1.91 s, 228 MiB |
| 100,000 | **4.5 s**, 262 MiB | 7.6 s, 488 MiB |

- **Exact against approximate.** Univo's search is exhaustive and returns every pair above the threshold. SemHash uses an approximate usearch index.
- **Different thresholds.** They remove different numbers of records (6,305 and 4,122 at 100,000), so this compares cost, not quality.
- **Raw results.** [scale-1.0.1.jsonl](../benchmarks/comparison/results/scale-1.0.1.jsonl), two runs of each tool at each size. Every change and its measurement is in [performance](development/performance.md).

Univo also deduplicates lexically, with no model: 100,000 names take **1.9 s** for synthetic names (same runs) and **2.2 s** for real ROR names. SemHash has no lexical mode.

## One record at a time

[check_or_add.py](../benchmarks/comparison/check_or_add.py) adds 100,000 of the same synthetic names to one partition, then checks 500 new ones with `check_or_add` at the default threshold of 0.8, timing each call. The model is loaded while adding, so the checks do not include it. Univo 1.0.1 from PyPI, Python 3.14, one thread, on mains power; two runs.

| Name field | Add 100,000 | Check one, median | Check one, p95 |
| --- | --- | --- | --- |
| Lexical | 0.14 s | 0.15 ms | 0.35–0.36 ms |
| Semantic, default model | 0.49 s | 1.31–1.33 ms | 1.36–1.38 ms |

A semantic check scans every vector of the partition, so it grows with the partition. A lexical check grows with the number of candidates that share trigrams with the new record.

## Name quality

**Corpus.** ROR name aliases, protocol `ror-alias-v2` ([manifest](../benchmarks/quality/ror-alias-v2/corpus.json)), built from the [ROR v2.14](https://zenodo.org/records/23187250) dump under CC0.
- **Entities.** 4,000 active organizations with aliases, chosen by hash, split by ROR ID into train (60%, unused), validation (20%) and test (20%).
- **Positives.** A display name and up to three of its aliases, labels or acronyms.
- **Negatives.** Neighbouring display names, a name halfway through the list, and organizations that ROR lists with the same acronym, all within the split.

**Against SemHash.** Each split's distinct names form one dataset of about 2,100 records; records sharing a ROR ID are duplicates. Both tools use the default model and SemHash's own decision rule ([dedupe.py](../benchmarks/comparison/dedupe.py)). The threshold of best F1 is chosen on validation, then applied once to test.

| Tool | Threshold | Test precision | Test recall |
| --- | --- | --- | --- |
| Univo, exact normalized | 1.00 | 6.7% | 0.1% |
| Univo, lexical | 0.51 | 40.3% | 19.3% |
| **Univo, semantic** | 0.63 | **62.5%** | **32.1%** |
| SemHash | 0.63 | 48.0% | 30.1% |

- **Same model, better decisions.** Univo adds acronym evidence and scores names on one template with distinct words lower, such as "Institute of Epidemiology" and "Institute of Oncology".
- **Equal names are not proof.** Most exact copies belong to different organizations.
- **No tool reaches 98% precision** on validation, so the high-precision rule abstains for all of them.

Raw results: [Univo](../benchmarks/comparison/results/ror-univo-semantic.json) ([lexical](../benchmarks/comparison/results/ror-univo-lexical.json), [exact](../benchmarks/comparison/results/ror-univo-exact.json)), [SemHash](../benchmarks/comparison/results/ror-semhash.json), with full curves.

**Calibration.** [evaluate_quality.py](../tools/quality/evaluate_quality.py) scores every pair with an exhaustive search. The threshold is the highest recall at precision ≥ 98% on validation, with at least 20 true positives; the test split is never used to choose it.

| Comparator | Validation threshold | Test precision | Test recall | Test AUC |
| --- | --- | --- | --- | --- |
| [Lexical](../benchmarks/quality/ror-alias-v2/lexical.json) | 0.590 | 95.6% | 14.5% | 0.461 |
| [Default model](../benchmarks/quality/ror-alias-v2/default-model.json) | 0.557 | 96.0% | 40.8% | 0.730 |

- **Precision without full recall.** Both stay near 96% precision, below the 98% precision and 90% recall targets set for automatic decisions. Version 1.0 therefore promises complete search and explained scores, not automatic decisions.
- **History.** Earlier scoring versions and models are in the same folders.

## Structured records

The [critical corpus](development/critical-corpus.md) has 222 valid pairs: organizations partitioned by tenant, products with a discriminant SKU, and configurations whose environment and parameters must agree. 44 pairs must never match: 32 contradict a discriminant field, such as `prod` against `staging`, and 12 belong to different tenants.

Univo is given schemas with partitions and vetoes. SemHash is given the same fields as columns, which it embeds together. [critical.py](../benchmarks/comparison/critical.py) reports each tool at its default threshold.

| Tool | Forbidden matches (of 44) | Duplicates found (of 93) | Distinct pairs matched (of 85) | AUC |
| --- | --- | --- | --- | --- |
| Univo, lexical, 0.8 | **0** | 56 | **9** | 0.815 |
| Univo, semantic, 0.8 | **0** | **64** | **9** | **0.833** |
| SemHash, 0.9 | 35 | 46 | 16 | 0.714 |

- **Hard rules.** Univo matches none of the 44 forbidden pairs at any threshold from 0.50 to 1.00. SemHash still matches 6 at 0.99: records that differ only by `prod` and `staging` have nearly equal embeddings.
- **Fairness.** SemHash does not claim to handle partitions or discriminant fields. A SemHash user would split records by tenant and filter vetoes in their own code.

Raw results: [Univo lexical](../benchmarks/comparison/results/critical-univo-lexical.json), [Univo semantic](../benchmarks/comparison/results/critical-univo-semantic.json), [SemHash](../benchmarks/comparison/results/critical-semhash.json).

## Footprint

| | Univo | SemHash |
| --- | --- | --- |
| Install | one 2.0 MiB wheel, 4.0 MiB installed, no Python dependency | 30 packages, 64 MiB installed |
| Model | 14.6 MiB, pinned revision and checksums, strict offline mode | downloaded at first use; offline needs a local path |
| Records | typed fields, IDs, partitions, vetoes, missing values | text columns |
| Index | add, upsert, remove, snapshots, `check_or_add` | rebuilt per call |

## Reproduce

Build the corpus from the ROR archive, whose SHA-256 the builder checks. Install SemHash 0.5.0 in its own environment; it is not a dependency of Univo.

```bash
python tools/quality/build_quality_corpus.py v2.14-2026-10-06-ror-data.zip --output /tmp/ror-pairs.jsonl
MODEL=/path/to/default-model     # tools/build_default_model.py, or the Hugging Face cache
python tools/quality/evaluate_quality.py /tmp/ror-pairs.jsonl --mode semantic --model-path $MODEL \
  --critical tests/fixtures/critical/cases.jsonl --output /tmp/quality.json
for tool in univo-exact univo-lexical univo-semantic; do
  python -I benchmarks/comparison/dedupe.py /tmp/ror-pairs.jsonl --tool $tool --model-path $MODEL --output /tmp/ror-$tool.json
done
semhash-env/bin/python -I benchmarks/comparison/dedupe.py /tmp/ror-pairs.jsonl --tool semhash --model-path $MODEL --output /tmp/ror-semhash.json
python -I benchmarks/comparison/critical.py tests/fixtures/critical/cases.jsonl --tool univo-semantic --model-path $MODEL --output /tmp/critical.json
python -I benchmarks/comparison/scale.py --tool univo-semantic --retrieval exact --records 100000 --model-path $MODEL
python -I benchmarks/comparison/check_or_add.py --model-path $MODEL
```

Run the timed commands one at a time, on mains power, with no other heavy workload. The [first comparison](../benchmarks/comparison/results/scale.jsonl), before parallel scoring, found SemHash 9 to 16 times faster; it led to parallel scoring and the blocked dense scan.
