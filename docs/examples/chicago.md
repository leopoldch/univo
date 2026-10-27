# Deduplicating Chicago's early childhood sites

Nine public lists of early childhood education sites in Chicago, from the city, the school district, the state and associations, describe the same places in different ways. Merged, they hold **3,337 rows for 1,162 real sites**. This is the dataset of the official [Dedupe](https://github.com/dedupeio/dedupe-examples) example, with the true site of every row labelled by its authors.

Univo groups the rows in under a tenth of a second, without labelling, training or a model.

<div class="tiles">
<div class="tile"><div class="figure">3,337</div><div class="caption">rows from 9 sources</div></div>
<div class="tile"><div class="figure">889</div><div class="caption">groups of proposed duplicates</div></div>
<div class="tile"><div class="figure">96%</div><div class="caption">of the pairs found are true duplicates, on sites held out from tuning</div></div>
<div class="tile"><div class="figure">0.1 s</div><div class="caption">to group every row, on a laptop</div></div>
</div>

## The data

One site, as six of the sources list it:

| Name | Address | Source |
| --- | --- | --- |
| Ezzard Charles School | 7946 S. Ashland | Chicago Public Schools portal |
| Ezzard Charles | 7946 South Ashland | Illinois child care providers |
| CHICAGO YOUTH CENTERS EZZARD CHARLES SCHOOL | 7946 S ASHLAND AVE | City providers, 2011 |
| Chicago Youth Centers Ezzard Charles School | 7946 S Ashland Ave | City agency sites, 2012 |
| Ezzard Charles | 7946 S Ashland | Find a School |
| Ezzard Charles School | 7946 South Ashland Avenue | NAEYC accreditation |

Case, abbreviations ("S." and "South", "Ave" and "Avenue") and the agency's name in front differ. The ZIP code is missing in 40% of the rows and the phone number in 4%.

## The code

```python
from univo import Exact, Finder, Schema, Text

schema = Schema(
    id_field="id",
    fields={
        "name": Text(),       # compared by spelling: typos, case, abbreviations
        "address": Text(),
        "phone": Exact(),     # compared only when both rows have one
        "zip": Exact(),
    },
)
groups = Finder(schema=schema).group(records, min_score=0.65)
```

Each group holds rows that all score 0.65 or more against each other. The [complete example](https://github.com/leopoldch/univo-examples/tree/main/clean-a-csv) is 22 lines of code; the data was cleaned beforehand, phone numbers reduced to their last seven digits.

<div class="viz-root" data-chart="bars"><script type="application/json">{"title": "From 3,337 rows to the sites they describe", "subtitle": "Rows if every proposed group is merged, against the true number of sites.", "format": "int", "valueName": "Rows", "labelWidth": 190, "items": [{"label": "Rows in the 9 sources", "value": 3337, "color": "ref"}, {"label": "After merging Univo's groups", "value": 1462, "color": 1, "note": "889 groups to review"}, {"label": "Real sites (labels)", "value": 1162, "color": "ref"}]}</script></div>

Merging every group would remove 1,875 rows; 2,175 rows duplicate a site. **Univo never merges anything itself.** A group is a proposal to review, because similarity is not identity: two branches of one agency can share a phone number.

## Choosing the threshold without fooling yourself

A threshold tuned on the data it is measured on looks better than it will be on new data. The sites were split in two by a hash of their label:

- on the **validation** half, three schemas and ten thresholds were tried, and the best recall with at least 95% precision was kept;
- the **test** half was measured once, with those settings.

<div class="viz-root" data-chart="xy"><script type="application/json">{"title": "Precision and recall by threshold, validation half", "subtitle": "Schema: name + address + phone + zip. The threshold kept is the best recall with at least 95% precision.", "x": {"label": "min_score", "min": 0.5, "max": 0.95, "format": "num2", "ticks": [0.5, 0.6, 0.7, 0.8, 0.9]}, "y": {"label": "Share of pairs", "min": 0.2, "max": 1.0, "format": "pct", "ticks": [0.2, 0.4, 0.6, 0.8, 1.0]}, "hover": "index", "rightPad": 80, "marker": {"x": 0.65, "label": "kept: 0.65"}, "series": [{"name": "Precision", "color": 1, "points": [{"x": 0.5, "y": 0.8662}, {"x": 0.55, "y": 0.9192}, {"x": 0.6, "y": 0.9418}, {"x": 0.65, "y": 0.9567}, {"x": 0.7, "y": 0.9721}, {"x": 0.75, "y": 0.9861}, {"x": 0.8, "y": 0.9915}, {"x": 0.85, "y": 0.9961}, {"x": 0.9, "y": 0.9976}, {"x": 0.95, "y": 0.999}]}, {"name": "Recall", "color": 2, "points": [{"x": 0.5, "y": 0.8955}, {"x": 0.55, "y": 0.8611}, {"x": 0.6, "y": 0.8254}, {"x": 0.65, "y": 0.772}, {"x": 0.7, "y": 0.701}, {"x": 0.75, "y": 0.6116}, {"x": 0.8, "y": 0.4985}, {"x": 0.85, "y": 0.4107}, {"x": 0.9, "y": 0.3336}, {"x": 0.95, "y": 0.2726}]}]}</script></div>

| Test half (1,602 rows, 2,885 true pairs) | Pairs found | Precision | Recall |
| --- | --- | --- | --- |
| Pairs scoring 0.65 or more (`find`) | 2,174 | 96.2% | 72.5% |
| Pairs inside groups (`group`) | 1,959 | 97.7% | 66.3% |

Groups are a little more precise than pairs and find fewer: a row joins a group only if it matches every member.

## Which fields to compare

Every field that identifies a site adds evidence. With the phone number and the ZIP code, recall at 95% precision rises from 69% to 77%.

<div class="viz-root" data-chart="xy"><script type="application/json">{"title": "At the same precision, phone and zip find more duplicates", "subtitle": "Validation half, one point per threshold from 0.5 to 0.95.", "x": {"label": "Recall", "min": 0.2, "max": 0.92, "format": "pct", "ticks": [0.2, 0.4, 0.6, 0.8]}, "y": {"label": "Precision", "min": 0.84, "max": 1.0, "format": "pct", "ticks": [0.85, 0.9, 0.95, 1.0]}, "hover": "nearest", "rightPad": 120, "series": [{"name": "name + address", "color": 1, "points": [{"x": 0.8437, "y": 0.8573, "label": "min_score 0.5"}, {"x": 0.7733, "y": 0.9424, "label": "min_score 0.55"}, {"x": 0.6922, "y": 0.9699, "label": "min_score 0.6"}, {"x": 0.6132, "y": 0.979, "label": "min_score 0.65"}, {"x": 0.5436, "y": 0.9912, "label": "min_score 0.7"}, {"x": 0.4668, "y": 0.9926, "label": "min_score 0.75"}, {"x": 0.4069, "y": 0.9928, "label": "min_score 0.8"}, {"x": 0.343, "y": 0.9915, "label": "min_score 0.85"}, {"x": 0.2973, "y": 0.9919, "label": "min_score 0.9"}, {"x": 0.2667, "y": 0.999, "label": "min_score 0.95"}], "label": "name + address", "labelAt": 0.8437, "labelDy": 16}, {"name": "name + address + phone", "color": 2, "points": [{"x": 0.8786, "y": 0.9109, "label": "min_score 0.5"}, {"x": 0.8442, "y": 0.9299, "label": "min_score 0.55"}, {"x": 0.8053, "y": 0.9478, "label": "min_score 0.6"}, {"x": 0.7381, "y": 0.9724, "label": "min_score 0.65"}, {"x": 0.6578, "y": 0.9839, "label": "min_score 0.7"}, {"x": 0.5697, "y": 0.993, "label": "min_score 0.75"}, {"x": 0.4811, "y": 0.9961, "label": "min_score 0.8"}, {"x": 0.3948, "y": 0.9973, "label": "min_score 0.85"}, {"x": 0.3229, "y": 0.9975, "label": "min_score 0.9"}, {"x": 0.2726, "y": 0.999, "label": "min_score 0.95"}], "label": "+ phone", "labelAt": 0.8786, "labelDy": 4}, {"name": "name + address + phone + zip", "color": 3, "points": [{"x": 0.8955, "y": 0.8662, "label": "min_score 0.5"}, {"x": 0.8611, "y": 0.9192, "label": "min_score 0.55"}, {"x": 0.8254, "y": 0.9418, "label": "min_score 0.6"}, {"x": 0.772, "y": 0.9567, "label": "min_score 0.65"}, {"x": 0.701, "y": 0.9721, "label": "min_score 0.7"}, {"x": 0.6116, "y": 0.9861, "label": "min_score 0.75"}, {"x": 0.4985, "y": 0.9915, "label": "min_score 0.8"}, {"x": 0.4107, "y": 0.9961, "label": "min_score 0.85"}, {"x": 0.3336, "y": 0.9976, "label": "min_score 0.9"}, {"x": 0.2726, "y": 0.999, "label": "min_score 0.95"}], "label": "+ phone + zip", "labelAt": 0.8955, "labelDy": 4}]}</script></div>

`Exact` fields count only when both rows have a value, so a missing ZIP code is no evidence either way. To make a difference rule out a match, such as two different tax numbers, use `Exact(conflict="reject")` ([scoring](../reference/scoring.md)).

## What it misses

- **Different phone numbers.** Some sources list the agency's number, others the site's, sometimes one digit apart.
- **Program names.** "Chicago State University Raise Up a Child" and "Chicago State University - Academy of Early Education" are one site at the same address, but their names and phone numbers differ.
- **Imperfect labels.** Some pairs counted as wrong look like the same site, such as "El Hogar del Niño New Loomis" and "El Hogar Loomis" at 1718 S. Loomis with the same phone number. Precision may be a little higher than measured.

## Run it

```bash
git clone https://github.com/leopoldch/univo-examples
cd univo-examples
pip install univo==1.0.1
python clean-a-csv/dedupe.py
```

The measurements of this page come from [benchmarks/chicago](../../benchmarks/chicago/README.md) in the Univo repository: `python -I benchmarks/chicago/chicago_quality.py`.

Measured with Univo 1.0.1 from PyPI, Python 3.14, Apple M2. The data is `csv_example_input_with_true_ids.csv` from [dedupeio/dedupe-examples](https://github.com/dedupeio/dedupe-examples/tree/48916bf09205756a2948a30ec0275ac2e394288d/csv_example) (MIT), pinned to a commit and checked by SHA-256.

Next: [best practices](../guide.md#best-practices).
