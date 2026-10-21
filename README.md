# Univo

**Compare anything. Keep every record clean.**

Fast, local duplicate detection for Python data: strings, records and nested objects.

[![PyPI](https://img.shields.io/pypi/v/univo)](https://pypi.org/project/univo/)
[![Python 3.11+](https://img.shields.io/badge/python-3.11%2B-blue)](https://github.com/leopoldch/univo/blob/main/pyproject.toml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](https://github.com/leopoldch/univo/blob/main/LICENSE)

```bash
pip install univo
```

```python
from univo import Finder

names = ["Northwind Traders", "Contoso", "Northwnd Traders", "NORTHWIND TRADERS", "Fabrikam"]
for match in Finder().find(names).matches:
    print(names[match.left_id], "~", names[match.right_id], round(match.score, 2))
```

```text
Northwind Traders ~ NORTHWIND TRADERS 1.0
Northwind Traders ~ Northwnd Traders 0.94
Northwnd Traders ~ NORTHWIND TRADERS 0.94
```

No server, no training, no API key. Records never leave your machine.

## Why Univo

- **Real records, not just strings.** Customers, products, organizations, configuration objects: dicts, lists and nested fields, compared field by field.
- **Rules before scores.** Two companies with different VAT numbers, or a `prod` and a `staging` config, never match, however alike they look.
- **Meaning, spelling and exact values.** Fuzzy text, a local multilingual model and exact comparison, chosen per field.
- **Every match explained.** Each match says which fields agreed and how. Univo never merges or deletes anything for you.
- **Fast and complete.** 100,000 names are deduplicated in 4.8 s, and search returns every pair above the threshold.
- **Light.** One 2 MiB wheel with no Python dependency. The 15 MB model downloads once, is verified, then works offline.

## Records and rules

A schema says which fields to compare, how, and which differences rule out a match. Nested fields are reached by path.

```python
from univo import Exact, Finder, Schema, Text

schema = Schema(
    id_field="id",
    fields={"name": Text(), "address.city": Text(), "vat": Exact(conflict="reject")},
)
finder = Finder(schema=schema)
finder.add(
    [
        {"id": 1, "name": "Northwind Traders SAS", "address": {"city": "LYON"}, "vat": "FR401"},
        {"id": 2, "name": "Northwind Traders", "address": {"city": "Lyon"}, "vat": "FR999"},
    ]
)
query = {"id": 3, "name": "Northwind Traders", "address": {"city": "Lyon"}, "vat": "FR401"}
for match in finder.search(query).matches:
    print(match.right_id, round(match.score, 2), match.reasons)
```

```text
1 0.94 ['name: trigrams', 'address.city: edit_distance', 'vat: exact']
```

Record 2 has another VAT number, so it can never match. For meaning rather than spelling, use `Text(mode="semantic")` ([models](https://github.com/leopoldch/univo/blob/main/docs/guide/models.md)).

## New records as they arrive

Keep an index and check each new record before you store it. Against 100,000 records, a check takes 0.15 ms on names, or 2.3 ms with embeddings.

```python
finder = Finder(
    schema=Schema(id_field="id", fields={"name": Text(), "vat": Exact(conflict="reject")})
)
finder.add([{"id": 1, "name": "Northwind Traders", "vat": "FR401"}])

finder.check_or_add({"id": 2, "name": "Northwnd Traders", "vat": "FR401"}).status  # "review"
finder.check_or_add({"id": 3, "name": "Contoso", "vat": "FR777"}).status  # "inserted"
finder.check_or_add({"id": 4, "name": "Northwind Traders", "vat": "FR401"}).status  # "matched"
```

`review` means a similar record exists and nothing was inserted: your application decides. Indexes can be updated, saved and loaded ([mutations](https://github.com/leopoldch/univo/blob/main/docs/guide/mutations.md), [snapshots](https://github.com/leopoldch/univo/blob/main/docs/guide/persistence.md)).

## Compared with other tools

| Tool | Built for | Choose Univo when |
| --- | --- | --- |
| [RapidFuzz](https://github.com/rapidfuzz/RapidFuzz) | Fast similarity between two strings | you want duplicate records, not string scores |
| [SemHash](https://github.com/MinishLab/semhash) | Semantic deduplication of text datasets | some fields must agree, such as IDs, tenants or environments |
| [Dedupe](https://github.com/dedupeio/dedupe) | Matching learned from labelled examples | you want results without labelling or training |
| [Splink](https://github.com/moj-analytical-services/splink) | Probabilistic record linkage at scale, on SQL engines | you want an in-process library that also checks records one at a time |

Against SemHash 0.5.0, with the same model, on an Apple M2:

| | Univo | SemHash |
| --- | --- | --- |
| Deduplicate 100,000 names, semantic | **4.8 s**, 270 MiB | 8.7 s, 491 MiB |
| Organization aliases (ROR), test precision / recall | **62.5% / 32.1%** | 48.0% / 30.1% |
| Structured records: forbidden matches, of 44 | **0** | 35 |
| Installed size | **4 MiB**, 1 package | 64 MiB, 30 packages |

Univo's search is exact; SemHash's index is approximate. On names alone, neither tool is precise enough to merge automatically, so Univo returns scored candidates, not decisions. Method and every result: [benchmarks](https://github.com/leopoldch/univo/blob/main/docs/benchmarks.md).

## Documentation

- **Guide.** [Quickstart](https://github.com/leopoldch/univo/blob/main/docs/guide/quickstart.md), [models and offline use](https://github.com/leopoldch/univo/blob/main/docs/guide/models.md), [mutations](https://github.com/leopoldch/univo/blob/main/docs/guide/mutations.md), [snapshots](https://github.com/leopoldch/univo/blob/main/docs/guide/persistence.md), [concurrency](https://github.com/leopoldch/univo/blob/main/docs/guide/concurrency.md), [groups](https://github.com/leopoldch/univo/blob/main/docs/guide/groups.md), [embedding cache](https://github.com/leopoldch/univo/blob/main/docs/guide/embedding-cache.md), [approximate search](https://github.com/leopoldch/univo/blob/main/docs/guide/ann.md).
- **Reference.** [Errors](https://github.com/leopoldch/univo/blob/main/docs/reference/errors.md), [scoring](https://github.com/leopoldch/univo/blob/main/docs/reference/scoring.md), [duplicate definitions](https://github.com/leopoldch/univo/blob/main/docs/reference/duplicate-definitions.md), [stability](https://github.com/leopoldch/univo/blob/main/docs/reference/stability.md).
- **Design.** [Search](https://github.com/leopoldch/univo/blob/main/docs/design/search.md), [lexical candidates](https://github.com/leopoldch/univo/blob/main/docs/design/lexical-search.md), [vector search](https://github.com/leopoldch/univo/blob/main/docs/design/vector-search.md), [exact index](https://github.com/leopoldch/univo/blob/main/docs/design/exact-index.md).
- **Development.** [Setup](https://github.com/leopoldch/univo/blob/main/docs/development/setup.md), [performance](https://github.com/leopoldch/univo/blob/main/docs/development/performance.md), [dependencies](https://github.com/leopoldch/univo/blob/main/docs/development/dependencies.md), [releasing](https://github.com/leopoldch/univo/blob/main/docs/development/releasing.md).
