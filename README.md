# Univo

**Compare anything. Keep every record clean.**

Find duplicates in Python data, from text datasets to structured records. Combine fuzzy matching, local embeddings and field-level rules. Fast, lightweight, no server required.

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

Text is compared by spelling by default; `Text(mode="semantic")` uses local embedding similarity ([models](https://github.com/leopoldch/univo/blob/main/docs/guide/models.md)). No training and no API key: records never leave your machine.

Built for data cleaning, entity resolution, ML dataset deduplication and real-time duplicate checks.

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

Record 2 has another VAT number, so it can never match.

## Why Univo

- **Records, not just strings.** Dicts, lists and nested fields, compared field by field, with an explanation for every match.
- **Rules before scores.** Different VAT numbers, tenants or environments rule out a match, however alike the records look.
- **Spelling, meaning and exact values**, chosen per field, with a 15 MB multilingual model that runs offline.
- **Fast, exact by default, and light.** 100,000 names deduplicated in 4.8 s. Search is exact by default, with explicit limits and optional approximate retrieval. One 2 MiB wheel, no Python dependency.

## New records as they arrive

Keep an index and check each new record before you store it.

```python
finder = Finder(
    schema=Schema(id_field="id", fields={"name": Text(), "vat": Exact(conflict="reject")})
)
finder.add([{"id": 1, "name": "Northwind Traders", "vat": "FR401"}])

finder.check_or_add({"id": 2, "name": "Northwnd Traders", "vat": "FR401"}).status  # "review"
finder.check_or_add({"id": 3, "name": "Contoso", "vat": "FR777"}).status  # "inserted"
finder.check_or_add({"id": 4, "name": "Northwind Traders", "vat": "FR401"}).status  # "matched"
```

`review` means a similar record exists and nothing was inserted: your application decides.

Checking one new record against 100,000 synthetic company names, in one partition, at the default threshold of 0.8, with the model already loaded (Apple M2, PyPI wheel, [script](https://github.com/leopoldch/univo/blob/main/benchmarks/comparison/check_or_add.py)):

| Name field | Median | p95 |
| --- | --- | --- |
| Lexical | 0.15 ms | 0.4 ms |
| With embeddings | 2.2 ms | 2.4 ms |

Times depend on your data: they grow with the records of a partition and with the number of close candidates. Indexes can be updated, saved and loaded ([mutations](https://github.com/leopoldch/univo/blob/main/docs/guide/mutations.md), [snapshots](https://github.com/leopoldch/univo/blob/main/docs/guide/persistence.md)).

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

[Quickstart](https://github.com/leopoldch/univo/blob/main/docs/guide/quickstart.md) · [Reference](https://github.com/leopoldch/univo/tree/main/docs/reference) · [Benchmarks](https://github.com/leopoldch/univo/blob/main/docs/benchmarks.md) · [Architecture](https://github.com/leopoldch/univo/tree/main/docs/design) · [All docs](https://github.com/leopoldch/univo/tree/main/docs)
