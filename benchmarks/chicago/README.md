# Chicago early childhood sites

The measurements behind the [Chicago walkthrough](../../docs/examples/chicago.md) and the [univo-examples](https://github.com/leopoldch/univo-examples) that use the same data.

```bash
python -I benchmarks/chicago/chicago_quality.py
```

```text
Chosen on validation: name + address + phone + zip, min_score 0.65
Test half, find pairs: precision 96.2%, recall 72.5%
Test half, pairs in groups: precision 97.7%, recall 66.3%
Whole dataset: 889 groups in 0.10 s
check_or_add decisions: {'inserted': 609, 'review': 1353, 'matched': 47}
  right: {'inserted': 0.625615763546798, 'review': 0.9571322985957132, 'matched': 1.0}
  median 0.13 ms, mean 0.14 ms
```

- **Data.** 3,337 rows describing 1,162 sites, from 9 public lists: `csv_example_input_with_true_ids.csv` of [dedupeio/dedupe-examples](https://github.com/dedupeio/dedupe-examples/tree/48916bf09205756a2948a30ec0275ac2e394288d/csv_example) (MIT), downloaded at a pinned commit and checked by SHA-256.
- **Split.** Sites are split in two halves by a hash of their label: every row of a site lands in the same half.
- **Choice.** On the validation half only: three schemas, thresholds from 0.5 to 0.95, the best recall with at least 95% precision.
- **Measure.** The test half, once. Precision and recall count pairs of rows; the whole-dataset groups are not measured against the labels, since half of them served to choose the threshold.
- **Incremental.** One list stored, the 2,009 rows of the 8 others checked one at a time with `check_or_add`. A decision is right when `inserted` goes to a site not stored yet, and `matched` or `review` to a site already stored.
- **Known limits.** Missed pairs mostly have different phone numbers. Some pairs counted as wrong look like the same site with a missing label, so precision may be slightly higher than measured.

Every number is in [results.json](results.json). Univo 1.0.1 from PyPI, Python 3.14, Apple M2.
