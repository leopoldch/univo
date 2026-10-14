# U-003: Univo against SemHash and lexical baselines

[SemHash](https://github.com/MinishLab/semhash) is the closest existing tool: local semantic deduplication with the same family of static models. [specs/01-product.md](../../specs/01-product.md) asks to run the same problem with SemHash and lexical baselines, and to measure quality, integration effort, time and memory, before generalizing. This report does that on three tasks. Nothing here changes Univo or chooses a threshold for it.

## Setup

- Apple M2 (8 cores), 16 GiB, macOS 27.0.1.
- Univo: release build (`maturin develop -r`) of `main` at U-018, CPython 3.14.7.
- SemHash 0.5.0 (model2vec 0.10.0, usearch 2.26.4, numpy 2.5.4), CPython 3.12.14, in a separate environment.
- **Same model for both:** `minishlab/potion-base-8M` at commit `bf8b0566`, read from the same local directory. SemHash downloads its model from the hub by default; it is given the local `StaticModel` instead, so both run offline with identical weights.
- **Isolation:** each run is a separate process, with the OS file cache warm. Peak RSS is the process maximum and includes inputs and Python.

## 1. Names: the ROR name-alias benchmark

The U-012 corpus ([quality README](../quality/README.md)) is rebuilt from ROR v2.14, whose MD5 and pinned SHA-256 were checked.

**Dataset.** Each split's distinct (ROR ID, name) records form a dataset: 2,130 validation records and 2,123 test records. Records sharing a ROR ID are duplicates. Their order is fixed by a hash of the record, so no tool chooses it.

**Decision rule, identical for every tool.** It is SemHash 0.5.0's own, which Univo's scored pairs go through in [dedupe.py](dedupe.py):
1. Records with exactly the same text form one group, in order of first occurrence. Every copy after the first is removed in favour of the first.
2. In that order, a group is removed when an earlier kept group reaches the threshold; all its records point to the first record of the most similar kept group, the earliest on a tie. Otherwise the group is kept.

A removal is correct when both records share a ROR ID. Recall counts every record beyond the first of its entity.

**The rule is checked against SemHash.** Fed with exact cosines of the same vectors, it makes the same removals as SemHash's exhaustive backend (`ann_backend="basic"`) at all 51 thresholds on test, and at 49 of 51 on validation. The two exceptions are one pair each, scored exactly at the threshold: "XDream Robotics (China)" against "XDream Robotics" at 0.8800002 for a threshold of 0.88, and "Maekawa Hoonkai" against "Maekawa Hōonkai" at 1.0. SemHash computes the similarity as one minus a float32 distance, which rounds both just below the threshold. With SemHash's default usearch index, the remaining differences come from approximate neighbours in dense clusters. [test_comparison.py](../../tests/tools/test_comparison.py) covers the rule, including `[X, A, Y, A]`: both `A` form one group, removed in favour of `X`; the second `A` never points to `Y`, kept in between, even when `Y` is closer.

**Thresholds.** They are chosen on validation with the U-012 rule (highest recall at precision ≥ 98% with at least 20 correct removals), then applied once to test. Because every tool abstains under that rule, the threshold of best validation F1 is also reported, applied to test without tuning on it.

| Tool | At 98% precision | Validation-F1 threshold | Test precision | Test recall | Time, both splits | Peak RSS |
| --- | --- | --- | --- | --- | --- | --- |
| Univo, exact normalized | abstains | 1.00 | 8.3% | 0.1% | 1.5 s (see below) | 42 MiB |
| Univo, lexical | abstains | 0.53 | 30.2% | 16.6% | 1.5 s | 44 MiB |
| Univo, semantic | abstains | 0.61 | 44.6% | 25.7% | **0.58 s** | 125 MiB |
| SemHash | abstains | 0.61 | 44.6% | 25.7% | 1.6 s | 147 MiB |

Findings:
- **Quality is the same on plain names.** The same model, the same cosine and the same rule give the same decisions: on test, both remove the same 772 records, 344 of them correctly. SemHash's approximate index can vary by a few records between runs.
- **The task is hard for every tool.** ROR aliases pair names such as "academyEX" and "The Mind Lab". Equal names are mostly homonyms: 11 of the 12 records removed as exact copies belong to another organization.
- **Univo is faster on this size,** 0.58 s against 1.6 s, model loading included. It also uses 15% less memory.
- **The exact baseline measures quality, not the cost of exact deduplication.** It runs the full lexical search at 0.5 and keeps the exact matches, so its 1.5 s is the lexical search's. Grouping by normalized text alone would take time linear in the records.

Raw results: [ror-univo-exact.json](results/ror-univo-exact.json), [ror-univo-lexical.json](results/ror-univo-lexical.json), [ror-univo-semantic.json](results/ror-univo-semantic.json), [ror-semhash.json](results/ror-semhash.json). Each has the full curve over thresholds 0.50 to 1.00.

## 2. Structured records: the critical corpus

The 222 valid critical pairs ([cases](../../tests/fixtures/critical/cases.jsonl)) are organizations partitioned by tenant, products with a discriminant SKU, and configurations whose service, environment and parameters must agree. **44 pairs must never match:**
- 32 contradict a discriminant field, for example `environment` prod against staging;
- 12 belong to different tenants.

**Inputs.**
- Univo is given the U-012 profile schemas: partitions, discriminant fields, a semantic description.
- SemHash is given the same fields as columns, which it embeds. It refuses `None`, so missing values become empty strings, and objects become JSON.

The corpus chooses no threshold; [critical.py](critical.py) reports each tool at its documented default.

| Tool, default threshold | Hard-rule violations (of 44) | Duplicates found (of 93) | Distinct pairs matched (of 85) | AUC |
| --- | --- | --- | --- | --- |
| Univo lexical, 0.8 | **0** | 58 | 23 | 0.755 |
| Univo semantic, 0.8 | **0** | 64 | 23 | **0.784** |
| SemHash, 0.9 | 35 (24 vetoes, 11 partitions) | 44 | 25 | 0.622 |

- **Hard rules.** Univo matches none of the 44 forbidden pairs at any threshold from 0.50 to 1.00. SemHash matches 35 at its default and still 7 at 0.99: an embedding of "prod" and an embedding of "staging" in otherwise identical records are nearly equal.
- **Ranking.** Univo ranks duplicates above distinct pairs better (AUC 0.784 against 0.622), because it compares fields one by one instead of one embedding of all fields.
- **Fairness.** SemHash does not claim to handle partitions or discriminant fields. A SemHash user can split records by tenant themselves; vetoes would need their own post-filter. That work is part of the comparison: Univo makes these rules declarative.

Raw results: [critical-univo-lexical.json](results/critical-univo-lexical.json), [critical-univo-semantic.json](results/critical-univo-semantic.json), [critical-semhash.json](results/critical-semhash.json), with sweeps over thresholds.

## 3. Time and memory at scale

[scale.py](scale.py) deduplicates synthetic company names in which one record in ten repeats an earlier name with a typo. Each tool runs with its documented defaults:
- SemHash at 0.9, with its usearch index;
- Univo at 0.8: lexical, or semantic with exhaustive dense retrieval (`--retrieval exact`) or the explicit HNSW retrieval of U-016 (`--retrieval hnsw`).

Univo's pairs go through the same greedy rule, inside the timed section. Thresholds differ, so the number of removed records differs: this measures cost, not quality. Both semantic retrievals of Univo remove the same number of records at every size.

**The same steps are timed for every tool.** Setup imports the tool and loads the model; Univo loads it lazily, so its setup runs a one-record search. Deduplication follows. The table gives the total wall time, then the CPU time of the deduplication over all threads, then the peak RSS.

| Records | SemHash | Univo semantic, exact | Univo semantic, HNSW | Univo lexical |
| --- | --- | --- | --- | --- |
| 10,000 | **1.0 s**, 3.5 s CPU, 162 MiB | 2.6 s, 2.6 s CPU, 129 MiB | 6.4 s, 6.4 s CPU, 145 MiB | 2.3 s, 2.1 s CPU, **41 MiB** |
| 30,000 | **2.5 s**, 13.7 s CPU, 284 MiB | 23 s, 23 s CPU, 169 MiB | 40 s, 40 s CPU, 216 MiB | 19 s, 19 s CPU, **75 MiB** |
| 100,000 | **14 s**, 85 s CPU, 643 MiB | 251 s, 251 s CPU, 308 MiB | 220 s, 220 s CPU, 472 MiB | 281 s, 281 s CPU, **177 MiB** |

Setup takes 0.2 to 0.4 s for SemHash and under 0.2 s for Univo; deduplication dominates every row.

**At scale, on a single-field semantic deduplication, SemHash is much faster: 9× at 30,000 records and 16× at 100,000, against Univo's faster retrieval at each size.** Three causes, measured in this table:
1. **Threads.** SemHash uses 5.5 to 6.2 cores on this 8-core machine; Univo's `find` scores on one thread. Per CPU-second, the gap is 1.7× at 30,000 records (exact retrieval) and 2.6× at 100,000 (HNSW).
2. **Retrieval for all-pairs work.** Exhaustive dense search is faster than the experimental U-016 HNSW defaults (`ann_ef_search=512`, `ann_max_candidates=1024`) up to 30,000 records, 23 s against 40 s, and slower only at 100,000, 251 s against 220 s. HNSW also costs 164 MiB more there.
3. **Completeness.** Univo's lexical lane is complete at its threshold, so a single partition costs close to quadratic time: 2.1 s at 10,000 records, 19 s at 30,000, 281 s at 100,000. Partitions and higher thresholds reduce it.

Univo uses less memory in every row: 41 MiB against 162 MiB at 10,000 records when lexical, and 308 MiB against 643 MiB at 100,000 records when semantic with exact retrieval.

Raw results: [scale.jsonl](results/scale.jsonl), one line per run.

## Integration effort

| | Univo | SemHash |
| --- | --- | --- |
| Install | one wheel, 1.6 MiB, no Python dependency | 34 packages, 60 MiB of site-packages (numpy, usearch, huggingface_hub, httpx…) |
| Model | pinned revision and checksums, verified cache, strict offline mode | downloaded from the hub at first use; offline needs an explicit local path |
| Records | typed fields, IDs, partitions, missing values as unknown | text columns; `None` refused; no IDs |
| Rules | exact normalization, discriminant fields (vetoes), partitions | none; user code |
| Output | scored pairs with per-field explanations; groups that never close chains | kept and removed records, one score each |
| Index | add, upsert, remove, snapshots, `check_or_add`, parallel searches | rebuilt per call |

## Conclusions

- **Univo improves concrete trade-offs.** It is correct on structured records where an embedding of all fields is not: 0 hard-rule violations against 35, and AUC 0.784 against 0.622. It is equal on plain names with the same model. It installs in 1.6 MiB with no dependency and uses less memory. The scope holds, per specs/01-product.md.
- **Univo is slower at all-pairs semantic deduplication of large single partitions.** Two measured improvements follow, before any release claim of speed:
  1. Parallelize all-pairs `find` and `group` across queries, within the `threads` budget. SemHash's advantage is mostly cores.
  2. Re-tune or bypass HNSW for all-pairs work. Up to 30,000 records, exhaustive search is faster than the current HNSW defaults, and it uses less memory at every size.
  3. Speed up the exhaustive kernel itself: per CPU-second, Univo remains 1.7× to 2.6× slower.

These results support no precision claim: the ROR task shows that no tool reaches 98% precision on name aliases alone.

## Reproduce

```bash
python tools/quality/build_quality_corpus.py /path/to/v2.14-2026-10-06-ror-data.zip --output /tmp/ror-pairs.jsonl
MODEL=/path/to/potion-base-8M   # config.json, model.safetensors, tokenizer.json at bf8b0566
for tool in univo-exact univo-lexical univo-semantic; do
  python -I benchmarks/comparison/dedupe.py /tmp/ror-pairs.jsonl --tool $tool --model-path $MODEL --output /tmp/ror-$tool.json
done
/path/to/semhash-env/bin/python -I benchmarks/comparison/dedupe.py /tmp/ror-pairs.jsonl --tool semhash --model-path $MODEL --output /tmp/ror-semhash.json
python -I benchmarks/comparison/critical.py tests/fixtures/critical/cases.jsonl --tool univo-semantic --model-path $MODEL --output /tmp/critical-univo-semantic.json
for records in 10000 30000 100000; do
  /path/to/semhash-env/bin/python -I benchmarks/comparison/scale.py --tool semhash --records $records --model-path $MODEL
  python -I benchmarks/comparison/scale.py --tool univo-lexical --records $records
  for retrieval in exact hnsw; do
    python -I benchmarks/comparison/scale.py --tool univo-semantic --retrieval $retrieval --records $records --model-path $MODEL
  done
done
```

Run the scale commands one at a time on an otherwise idle machine; each prints one line of [scale.jsonl](results/scale.jsonl).

Univo needs an installed wheel or the development environment. SemHash needs `pip install semhash==0.5.0` in a separate environment; it is not a dependency of Univo. The scripts use only the standard library, plus the tool being measured.
