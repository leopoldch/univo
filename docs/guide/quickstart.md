# Quickstart

## Find duplicates in a list

```python
from univo import Finder

finder = Finder(offline=True)
report = finder.find(["Northwind Traders", "Northwnd Traders", "Contoso"])
for match in report.matches:
    print(match.left_id, match.right_id, match.kind, match.score)
```

This prints the Northwind pair. Plain strings are compared lexically and need no model. A score ranks how alike two records are. It is not a probability, and the default `min_score` of 0.8 is not calibrated.

## Records

A schema says which fields to compare, how, and which records can never match.

```python
from univo import Exact, Finder, Schema, Text

schema = Schema(
    id_field="id",
    partition_by=["tenant"],
    fields={
        "name": Text(),
        "environment": Exact(conflict="reject"),
    },
)
finder = Finder(schema=schema, offline=True)
records = [
    {"id": "org-1", "tenant": "shop", "name": "Northwind Traders", "environment": "prod"},
    {"id": "org-2", "tenant": "shop", "name": "NORTHWIND TRADERS", "environment": "prod"},
    {"id": "org-3", "tenant": "shop", "name": "Northwind Traders", "environment": "test"},
]
finder.add(records)
report = finder.search(records[0])
assert [match.right_id for match in report.matches] == ["org-2"]
```

- **`Text`** compares text. `mode` is `"hybrid"` by default, or `"lexical"` or `"semantic"`.
- **`Exact`** compares values as they are. `conflict="reject"` makes a difference a veto: `org-3` never matches.
- **`partition_by`** limits matches to records with the same values.
- **`id_field`** gives records their IDs. Without it, records get integer slots.

Compared values can be strings, signed 64-bit integers, finite floats, booleans, `None`, lists, and dicts with string keys. Fields outside the schema are ignored, and inputs are never modified.

A schema also bounds each record: 64 fields, 1 MiB of compared values, 16 levels of nesting. Raise them with `max_fields`, `max_selected_bytes` and `max_depth` (at most 128).

## Keep an index

`add`, `upsert` and `remove` change the records of a `Finder`, and `search` queries them. `find` and `group` work on a temporary set and leave the index unchanged.

`check_or_add(record)` searches and adds in one step. It returns `inserted`, `matched` or `review`, and never merges anything itself ([concurrency](concurrency.md)).

## Semantic fields

```python
from univo import Finder, Schema, Text

finder = Finder(schema=Schema(fields={"description": Text(mode="semantic")}))
```

The default model is downloaded on first use, verified and cached. With `offline=True`, Univo uses only the cache or a local model and never connects. [Models](models.md) covers other models and mirrors.

## Limits and reports

Every result says whether it is complete:

| Attribute | Meaning |
| --- | --- |
| `matches` | The matches, best first, up to `limit` |
| `total_matches` | All the matches found, including those beyond `limit` |
| `output_truncated` | More matches were found than returned |
| `candidate_truncated` | A work budget ran out: matches may be missing |
| `search_profile` | `exhaustive`, `bounded` or `approximate` |

The main options:

| Option | Default | |
| --- | --- | --- |
| `min_score` | `0.8` | Lowest score returned |
| `limit` | `1000` for `find`, `10` for `search` | Matches returned |
| `exhaustive` | `False` | Compare every pair instead of using candidate search |
| `threads` | every core | Threads for `find`, `group` and encoding |
| `max_records` | `100000` | Records per index |
| `max_exhaustive_pairs` | `1000000` | Pairs an exhaustive `find` may compare |
| `max_semantic_comparisons` | `1000000` | Vector comparisons a semantic `find` may make |

Limits act in two ways:
- **Truncation.** Reaching `limit` cuts the results and sets `output_truncated`. A candidate lane that runs out of work sets `candidate_truncated`.
- **Errors.** Going over `max_records`, `max_exhaustive_pairs` or `max_semantic_comparisons` raises `ResourceLimitError`, without a partial result. Raise the limit explicitly for larger jobs.

These limits bound each operation, not the memory of the process: large records or indexes can still need more memory than is available. Errors have a stable `code` and a `hint` ([errors](../reference/errors.md)).

## Next

[Mutations](mutations.md) · [Snapshots](persistence.md) · [Groups](groups.md) · [Concurrency](concurrency.md) · [Approximate search](ann.md) · [Scoring](../reference/scoring.md)
