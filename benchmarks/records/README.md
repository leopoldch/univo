# Record datasets

Univo on five public datasets of records with several fields and known duplicates, next to [Splink](https://github.com/moj-analytical-services/splink) 5.0.0 where Splink publishes its own settings for the dataset.

| Dataset | Task | Records | True pairs | Source |
| --- | --- | --- | --- | --- |
| `febrl3` | Deduplicate synthetic persons, up to five altered copies each | 5,000 | 6,538 | Febrl, from [splink_datasets](https://github.com/moj-analytical-services/splink_datasets) |
| `febrl4` | Link 5,000 synthetic persons to one altered copy of each | 10,000 | 5,000 | Febrl, same repository |
| `historical_50k` | Deduplicate historical persons from Wikidata, with errors and missing values, about ten records per person | 50,578 | 303,961 | same repository |
| `dblp_acm` | Link publications of DBLP and of the ACM Digital Library | 4,910 | 2,224 | [Leipzig benchmark datasets](https://dbs.uni-leipzig.de/research/projects/benchmark-datasets-for-entity-resolution) |
| `abt_buy` | Link products of two shops, described in free text | 2,173 | 1,097 | same site |

No file is redistributed here: each is downloaded from its publisher at a pinned location and checked by SHA-256 ([record_datasets.py](record_datasets.py)).

## Results

Test half of each dataset, with the schema and the threshold chosen on the validation half. Both Univo modes are measured on one build, the one that adds `min_probability`: by score it writes the pairs and scores of 1.0.2 on the five datasets; by probability is `find(min_probability=...)`, weights learned from the records without labels. Python 3.14; Splink 5.0.0 on DuckDB, Python 3.12; Apple M2.

| Dataset | Tool | Pairs: precision / recall | Pairs: F1 | Entities: F1 (precision / recall) | Seconds |
| --- | --- | --- | --- | --- | --- |
| `febrl3` | Univo, by score | 100.0% / 99.7% | 0.998 | 0.999 (100.0% / 99.8%) | 0.7 |
| | Univo, `group` | | | 0.986 (100.0% / 97.2%) | 0.3 |
| | Univo, by probability | 100.0% / 99.9% | **1.000** | **1.000** (100.0% / 100.0%) | 0.7 |
| | Splink | 99.8% / 99.2% | 0.995 | 0.998 (100.0% / 99.6%) | 0.8 |
| `febrl4` | Univo, by score, `search` | 99.9% / 100.0% | 0.999 | linking | 7.2 |
| | Univo, by probability | 100.0% / 100.0% | **1.000** | linking | **0.9** |
| | Splink | 100.0% / 99.9% | 0.999 | linking | 2.0 |
| `historical_50k` | Univo, by score | 86.8% / 72.6% | 0.791 | 0.581 (55.5% / 61.0%) | 16.6 |
| | Univo, `group` | | | 0.713 (94.0% / 57.4%) | 11.2 |
| | Univo, by probability | 92.4% / 83.9% | **0.879** | **0.922** (96.4% / 88.4%) | 4.8 |
| | Splink | 93.6% / 65.9% | 0.773 | 0.840 (90.3% / 78.5%) | **3.7** |
| `dblp_acm` | Univo, by score, `search` | 98.2% / 97.8% | **0.980** | linking | 0.8 |
| | Univo, by probability | 95.2% / 97.3% | 0.962 | linking | 0.4 |
| `abt_buy` | Univo, by score, `search` | 57.5% / 51.9% | **0.546** | linking | under 0.1 |
| | Univo, by probability | 73.0% / 33.3% | 0.457 | linking | 0.3 |

- **Pairs.** Every pair scoring at least the threshold, against the true pairs.
- **Entities.** What a deduplicated table needs: the pairs kept at a threshold are closed transitively into groups, and every pair of records of one group counts as found. One wrong pair then joins two entities. `group` is Univo's own grouping, which joins records only when every pair of the group reaches the threshold: it is measured as it is, without closing.
- **Seconds.** From the records in memory to the scored pairs in memory, on the test half. For Splink and for Univo by probability this includes learning the weights. By score it is the time at the lowest score written, 0.5, or 0.6 on `historical_50k`; by probability, at a minimum of 0.05. One run each, on mains, in one session: an order of magnitude, not a measurement to publish.

**What this shows.**

- **Clean records: no difference.** On Febrl, where most fields are present and altered copies stay close, every tool finds nearly every pair.
- **By score, entities on `historical_50k` are far behind.** With ten records per person, a deduplicated table depends on chaining pairs. The score is a mean of field similarities: its wrong pairs score high enough to join entities, so closing them drops precision to 55%, and `group`, which does not chain, keeps precision and loses recall.
- **By probability, Univo passes Splink there**, on pairs and on entities, with the same kind of model. On that half it takes 4.8 s, against 16.6 s by score and 3.7 s for Splink.
- **A well chosen schema still wins on easy linking.** On `dblp_acm` the score with title, authors and year reaches 0.980; weights reach 0.962.
- **Free text: weak either way.** On `abt_buy` the product names and descriptions carry everything; weights have nothing to weigh.

**Probabilities against labels**, test half of `historical_50k`, schema with every field:

| Probability given | Pairs | Mean given | True |
| --- | --- | --- | --- |
| 0.05 to 0.5 | 43,069 | 0.164 | 0.299 |
| 0.5 to 0.9 | 13,622 | 0.726 | 0.792 |
| 0.9 to 0.99 | 14,961 | 0.961 | 0.960 |
| 0.99 to 0.999 | 14,789 | 0.996 | 0.995 |
| 0.999 and more | 78,623 | 1.000 | 1.000 |

Above 0.9 the probability given is the share of true pairs; below, it understates it.

The spike [field-weights](../spikes/field-weights/results.md) is the prototype this mode came from.

## Method

- **Split.** Records are split in two halves by a hash of their entity, so every record of an entity, and both sides of a true link, land in the same half. For the Leipzig datasets an entity is a connected set of mapped records.
- **Choice on validation.** Each tool gets, per rule, its variant and threshold on the validation half; the test half is measured once with them. The rules are the best F1 on pairs, the best recall with at least 95% precision ([results.json](results.json)), and the best F1 on entities, over 60 quantiles of the scores.
- **Univo.** A few schemas per dataset, written from the meaning of the columns ([record_pairs_univo.py](record_pairs_univo.py)), each ranked by its score and by its score times its coverage. Deduplication calls `find`. Linking adds the first source and calls `search` for each record of the second, keeping ten matches.
- **Splink.** The comparisons, blocking rules and training steps of its own example notebooks for these three datasets, at commit `f319070e` ([record_pairs_splink.py](record_pairs_splink.py)). It learns its parameters from the records it scores, without labels, so each half is trained and scored on its own. Pairs are ranked by match weight.
- **True pairs.** For deduplication, every pair of records of one entity. For linking, the pairs of the published mapping.

## Reproduce

Splink is installed in its own environment; it is not a dependency of Univo. `historical_50k` needs `pyarrow` to be read once, which that environment has.

```bash
DATA=/tmp/records-data PAIRS=/tmp/records-pairs
splink-env/bin/python benchmarks/records/record_datasets.py --cache $DATA
MODEL=/path/to/default-model     # for the semantic schemas of dblp_acm and abt_buy
for d in febrl3 febrl4 dblp_acm abt_buy; do
  python -I benchmarks/records/record_pairs_univo.py --dataset $d --cache $DATA --output $PAIRS/$d-univo.json --model-path $MODEL
done
python -I benchmarks/records/record_pairs_univo.py --dataset historical_50k --floor 0.6 --cache $DATA --output $PAIRS/historical_50k-univo.json
for d in febrl3 historical_50k; do
  python -I benchmarks/records/record_pairs_univo.py --group --dataset $d --cache $DATA --output $PAIRS/$d-univo-group.json
done
for d in febrl3 febrl4 dblp_acm abt_buy historical_50k; do
  python -I benchmarks/records/record_pairs_univo.py --probability --dataset $d --cache $DATA --output $PAIRS/$d-univo-probability.json --model-path $MODEL
done
for d in febrl3 febrl4 historical_50k; do
  splink-env/bin/python -I benchmarks/records/record_pairs_splink.py --dataset $d --cache $DATA --output $PAIRS/$d-splink.json
done
python -I benchmarks/records/record_quality.py --cache $DATA --pairs $PAIRS --output benchmarks/records/results.json
```

## Limits

- **Splink only where it publishes settings.** For `dblp_acm` and `abt_buy` there is no Splink example, and settings written here would not be a fair reference.
- **Dedupe is not measured.** It learns from labelled pairs, which needs its own protocol.
- **Other published figures use other protocols**, often a classifier trained on labelled candidate pairs: they do not compare with these.
- **Times** are single runs.
- **The validation half chose schemas and thresholds**, for Univo by score among 6 to 8 variants per dataset, by probability among 3 or 4, for Splink the threshold only.
- **Linking by probability** calls `find` on both sources together and keeps the pairs across them: chance agreement is then counted inside each source too.
- **The probability table is one dataset**, and a synthetic one.
