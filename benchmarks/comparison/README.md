# U-003: Univo against SemHash and lexical baselines

[SemHash](https://github.com/MinishLab/semhash) is the closest existing tool: local semantic deduplication with the same family of static models. [specs/01-product.md](../../specs/01-product.md) asks to run the same problem with SemHash and lexical baselines, and to measure quality, integration effort, time and memory, before generalizing. This report does that on three tasks. Nothing here changes Univo or chooses a threshold for it.

## Setup

- Apple M2 (8 cores), 16 GiB, macOS 27.0.1.
- Univo: development build of `main` at U-018, CPython 3.14.7.
- SemHash 0.5.0 (model2vec 0.10.0, usearch 2.26.4, numpy 2.5.4), CPython 3.12.14, in a separate environment.
- **Same model for both:** `minishlab/potion-base-8M` at commit `bf8b0566`, read from the same local directory. SemHash downloads its model from the hub by default; it is given the local `StaticModel` instead, so both run offline with identical weights.
- **Isolation:** each run is a separate process, with the OS file cache warm. Peak RSS is the process maximum and includes inputs and Python.

## 1. Names: the ROR name-alias benchmark

The U-012 corpus ([quality README](../quality/README.md)) is rebuilt from ROR v2.14, whose MD5 and pinned SHA-256 were checked.

**Dataset.** Each split's distinct (ROR ID, name) records form a dataset: 2,130 validation records and 2,123 test records. Records sharing a ROR ID are duplicates. Their order is fixed by a hash of the record, so no tool chooses it.

**Decision rule, identical for every tool.** It is SemHash's own: in input order, a record is removed when an earlier kept record reaches the threshold, and it points to the most similar one. Univo returns scored pairs; [dedupe.py](dedupe.py) applies the same rule to them. A removal is correct when both records share a ROR ID. Recall counts every record beyond the first of its entity.

**Thresholds.** They are chosen on validation with the U-012 rule (highest recall at precision ≥ 98% with at least 20 correct removals), then applied once to test. Because every tool abstains under that rule, the threshold of best validation F1 is also reported, applied to test without tuning on it.

| Tool | At 98% precision | Validation-F1 threshold | Test precision | Test recall | Time, both splits | Peak RSS |
| --- | --- | --- | --- | --- | --- | --- |
| Univo, exact normalized | abstains | 1.00 | 8.3% | 0.1% | 1.4 s | 42 MiB |
| Univo, lexical | abstains | 0.53 | 30.2% | 16.6% | 1.5 s | 44 MiB |
| **Univo, semantic** | abstains | 0.61 | **44.6%** | **25.7%** | **0.47 s** | 125 MiB |
| SemHash | abstains | 0.61 | 44.3% | 25.3% | 1.8 s | 147 MiB |

Findings:
- **Quality is the same on plain names.** The same model, the same cosine and the same rule give the same quality. SemHash's approximate index varies by a few records between runs (25.3% to 25.4% recall).
- **The task is hard for every tool.** ROR aliases pair names such as "academyEX" and "The Mind Lab". Equal names are mostly homonyms: 11 of the 12 records removed as exact copies belong to another organization.
- **Univo is faster on this size,** 0.47 s against 1.8 s, model loading included. It also uses 15% less memory.

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
- Univo at 0.8, lexical with exact retrieval, or semantic with the explicit HNSW retrieval of U-016.

Univo's pairs go through the same greedy rule, inside the timed section. Thresholds differ, so the number of removed records differs: this measures cost, not quality.

| Records | SemHash | Univo semantic (HNSW) | Univo lexical |
| --- | --- | --- | --- |
| 10,000 | 0.7 s, 163 MiB | 6.5 s, 145 MiB | 2.1 s, **41 MiB** |
| 30,000 | **2.5 s**, 283 MiB | 43 s, 221 MiB | 18.8 s, 75 MiB |
| 100,000 | **14 s**, 645 MiB | 229 s, 505 MiB | not run (about 3.5 min expected) |

**At scale, on a single-field semantic deduplication, SemHash is much faster: 17× at 30,000 records, 16× at 100,000.** Three causes, measured:
1. **Threads.** SemHash uses several cores: 13.0 s of CPU for 2.9 s of wall time at 30,000 records, about 4.4 cores. Univo's `find` scores on one thread: 43.3 s of CPU for 43.4 s of wall time. Per CPU-second, the gap is 3.3×.
2. **HNSW tuning.** At 10,000 records, Univo's exhaustive semantic search finds the same 987 pairs as its HNSW mode in 2.6 s instead of 6.5 s. The experimental U-016 defaults (`ann_ef_search=512`, `ann_max_candidates=1024`) cost more than they save on all-pairs work at this size.
3. **Completeness.** Univo's lexical lane is complete at its threshold, so a single partition costs close to quadratic time: 2.1 s at 10,000 records, 18.8 s at 30,000. Partitions and higher thresholds reduce it.

Univo uses less memory in every row: 41 MiB against 163 MiB at 10,000 records when lexical, and 505 MiB against 645 MiB at 100,000 records when semantic.

Raw results: [scale.jsonl](results/scale.jsonl).

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
  2. Re-tune or bypass HNSW for all-pairs work. At 10,000 records, exhaustive search is faster than the current HNSW defaults.

These results support no precision claim: the ROR task shows that no tool reaches 98% precision on name aliases alone.

## Reproduce

```bash
python tools/build_quality_corpus.py /path/to/v2.14-2026-10-06-ror-data.zip --output /tmp/ror-pairs.jsonl
MODEL=/path/to/potion-base-8M   # config.json, model.safetensors, tokenizer.json at bf8b0566
for tool in univo-exact univo-lexical univo-semantic; do
  python -I benchmarks/comparison/dedupe.py /tmp/ror-pairs.jsonl --tool $tool --model-path $MODEL --output /tmp/ror-$tool.json
done
/path/to/semhash-env/bin/python -I benchmarks/comparison/dedupe.py /tmp/ror-pairs.jsonl --tool semhash --model-path $MODEL --output /tmp/ror-semhash.json
python -I benchmarks/comparison/critical.py tests/fixtures/critical/cases.jsonl --tool univo-semantic --model-path $MODEL --output /tmp/critical-univo-semantic.json
python -I benchmarks/comparison/scale.py --tool univo-lexical --records 10000
```

Univo needs an installed wheel or the development environment. SemHash needs `pip install semhash==0.5.0` in a separate environment; it is not a dependency of Univo. The scripts use only the standard library, plus the tool being measured.
