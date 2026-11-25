# Field weights learned without labels

2026-11-25, base `33d2a33`, Apple M2. **A Python prototype over Univo's own candidates and field similarities. It changes nothing in Univo; production scoring remains version 4.**

## Question

On [`historical_50k`](../../records/README.md), Univo finds pairs about as well as Splink, but the entities it builds from them are far worse: F1 0.58 to 0.71 against 0.84. Is the cause the way field similarities are combined, a weighted mean, and would weights learned from the records themselves, as probabilistic record linkage does, repair it without labels?

## Protocol

[field_weights.py](field_weights.py) keeps what Univo computes today and replaces only the combination.

- **Candidates and similarities.** `find`, or `search` for linking, with one schema per dataset holding every useful column, at a lowest score of 0.5, 0.6 on `historical_50k`. Each candidate comes with Univo's field similarities.
- **Levels.** A similarity is read as equal, close (at least 0.88), partial (at least 0.7) or different. These limits were set once, before any run, for every field and dataset.
- **u**, the chance of a level between different entities: counted over every pair of 1,500 records drawn at random. For equal values, the frequency of the value replaces it.
- **m**, the chance of a level inside an entity: the candidates are split in two classes by expectation maximisation, and the class that agrees gives m.
- **Weight** of a pair: the sum over its fields of log2(m / u). A missing value adds nothing.
- **No label is read** to learn. Thresholds are chosen on the validation half and measured once on the test half, by [record_quality.py](../../records/record_quality.py), as for the other tools.

## Results

Test halves. Both columns rank the **same candidates**, by Univo's score and by the learned weights, each with its own threshold from validation.

| Dataset | Pairs F1, Univo's score | Pairs F1, learned weights | Entities F1, Univo's score | Entities F1, learned weights |
| --- | --- | --- | --- | --- |
| `febrl3` | 0.998 | **1.000** | 0.999 | 0.999 |
| `febrl4` | 0.999 | **1.000** | linking | linking |
| `historical_50k` | 0.689 | **0.867** | 0.002 | **0.926** |
| `dblp_acm` | 0.941 | **0.975** | linking | linking |
| `abt_buy` | **0.424** | 0.387 | linking | linking |

Against the tools of the [record benchmark](../../records/README.md), on `historical_50k`:

| | Pairs: precision / recall | Pairs: F1 | Entities: F1 (precision / recall) |
| --- | --- | --- | --- |
| Univo 1.0.2, best schema on validation | 86.8% / 72.6% | 0.791 | 0.581, or 0.713 with `group` |
| Splink 5.0.0, its example settings | 93.6% / 65.9% | 0.773 | 0.840 (90.3% / 78.5%) |
| Univo's candidates, learned weights | 93.1% / 81.1% | **0.867** | **0.926** (96.9% / 88.7%) |

Weights learned on the test half of `historical_50k`, in bits:

| Field | Equal | Close | Partial | Different |
| --- | --- | --- | --- | --- |
| `first_name` | +4.8 | +6.4 | +4.9 | −1.0 |
| `surname` | +9.8 | +9.9 | +7.6 | −3.1 |
| `dob` | +7.7 | +6.4 | +3.9 | −2.1 |
| `postcode_fake` | +10.9 | +7.0 | +4.9 | −1.4 |
| `birth_place` | +6.9 | +7.4 | +3.2 | −2.9 |
| `occupation` | +4.7 | | | −3.5 |

Equal values are then adjusted by the frequency of the value.

## Reading

- **The combination was the cause.** With the same candidates and the same field similarities, learned weights raise the entities F1 on `historical_50k` from 0.002 to 0.926. With Univo's score, the threshold chosen on validation lets one group swallow 11,707 records of the test half; with the weights the largest group holds 40.
- **More true pairs reach the ranking.** The pairs written here, those scoring at least 0.6, hold 85.5% of the true pairs of that half; the pairs Splink wrote, at −10 bits or more, hold 66.6%. These are pairs written after two different floors: the recall of the two candidate searches themselves was not measured. With weights on top, the result passes Splink's on pairs and on entities.
- **No loss on clean records**, and a gain on `dblp_acm` with the same schema. The best schema of the record benchmark, without the venue, reaches 0.980 there with Univo's score: weights make the choice of fields matter less, they do not beat a well chosen schema on easy data.
- **No help on free text.** On `abt_buy`, two semantic fields and a price carry too little that levels can separate; the weights lose to the score, and both lose to the name alone (0.546).

## Limits

- **A prototype in Python**: 50 s on the test half of `historical_50k`, of which 17 s find the candidates. It says nothing of the cost of an implementation in the engine.
- **One dataset carries the conclusion.** `historical_50k` is synthetic: errors were introduced by a program. The gain must be confirmed on records with real errors.
- **Choices fixed here, not studied**: four levels and their limits, the lowest score for candidates, 1,500 sampled records, independence of fields. Two fields that agree together, such as a name and a full name, would be counted twice.
- **Linking counts chance over pairs of both sources together**, which misreads values that each source writes its own way: on `dblp_acm` an equal venue weighs −10 bits.
- **Levels with few observations get arbitrary weights**: a level seen in neither class gets the ratio of two floors.
- **The validation half has chosen thresholds only**, but the datasets, the levels and the protocol were all looked at during this work: these are not held-out results in the strict sense.

Every number is in [results.json](results.json); reproduce with the commands of the [record benchmark](../../records/README.md#reproduce), then:

```bash
for d in febrl3 febrl4 dblp_acm abt_buy; do
  python -I benchmarks/spikes/field-weights/field_weights.py --dataset $d --cache $DATA --output /tmp/records-weights/$d-weights.json --model-path $MODEL
done
python -I benchmarks/spikes/field-weights/field_weights.py --dataset historical_50k --floor 0.6 --cache $DATA --output /tmp/records-weights/historical_50k-weights.json
python -I benchmarks/records/record_quality.py --cache $DATA --pairs /tmp/records-weights --output benchmarks/spikes/field-weights/results.json
```
