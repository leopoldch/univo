# Univo

**Compare anything. Keep every record clean.**

Fast, local duplicate detection for Python data: strings, records and nested objects.

[![PyPI](https://img.shields.io/pypi/v/univo)](https://pypi.org/project/univo/)
[![Python 3.11+](https://img.shields.io/badge/python-3.11%2B-blue)](https://github.com/leopoldch/univo/blob/main/pyproject.toml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](https://github.com/leopoldch/univo/blob/main/LICENSE)

Univo finds duplicates in any structured Python data: customers, products, organizations, configuration objects. You say which fields to compare and how: fuzzy text, meaning, exact values, nested paths. You also say which fields must agree and which records belong together. Univo then compares records field by field and explains every match.

A Rust engine does the work behind a small Python API. Inference is local: no record leaves your machine.

- **Fast.** 100,000 names are deduplicated in 4.8 s with embeddings and in 1.9 s lexically, on a laptop.
- **Complete.** Search returns every pair above the threshold. A work or output limit, when reached, is flagged in the report ([limits](https://github.com/leopoldch/univo/blob/main/docs/guide/quickstart.md#limits-and-reports)). Approximate search is opt-in.
- **Any structure.** Records are dicts, lists, strings, numbers, booleans and `None`, nested within configurable limits ([supported values](https://github.com/leopoldch/univo/blob/main/docs/guide/quickstart.md#records)). Dict keys compare in any order.
- **Rules first.** Partitions, typed IDs and vetoes, such as two different VAT numbers, apply before any score.
- **Explained.** Each match lists its evidence per field. Univo never merges or deletes records for you.
- **Light.** One 2 MiB wheel with no Python dependency, and a 15 MB multilingual model that is verified, cached and usable strictly offline.

## Benchmarks

Against [SemHash](https://github.com/MinishLab/semhash) 0.5.0, with the same model, on an Apple M2:

| | Univo | SemHash |
| --- | --- | --- |
| Deduplicate 100,000 names, semantic | **4.8 s**, 270 MiB | 8.7 s, 491 MiB |
| Organization aliases (ROR), test precision / recall | **62.5% / 32.1%** | 48.0% / 30.1% |
| Structured records: forbidden matches, of 44 | **0** | 35 |
| Installed size | **4 MiB**, 1 package | 64 MiB, 30 packages |

Univo's search is exact, while SemHash's index is approximate. Neither tool reaches the precision needed for automatic merging on names alone, so Univo reports scored candidates, not decisions. Method, limits and every result: [benchmarks](https://github.com/leopoldch/univo/blob/main/docs/benchmarks.md).

## Install

```bash
pip install univo
```

Wheels are built for Linux, macOS and Windows, on x86_64 and arm64, for Python 3.11 and later. Rust is not needed. To build from source, see [contributing](https://github.com/leopoldch/univo/blob/main/CONTRIBUTING.md).

## Quickstart

```python
from univo import Finder

report = Finder(offline=True).find(["Northwind Traders", "Northwnd Traders", "Contoso"])
for match in report.matches:
    print(match.left_id, match.right_id, round(match.score, 2), match.reasons)
```

Structured records take a schema: an ID, and how each field compares. Nested fields are reached by path.

```python
from univo import Exact, Finder, Schema, Text

schema = Schema(
    id_field="id",
    fields={"name": Text(), "address.city": Text(), "vat": Exact(conflict="reject")},
)
finder = Finder(schema=schema, offline=True)
finder.add(
    [
        {"id": 1, "name": "Northwind Traders SAS", "address": {"city": "LYON"}, "vat": "FR401"},
        {"id": 2, "name": "Northwind Traders", "address": {"city": "Lyon"}, "vat": "FR999"},
    ]
)
query = {"id": 3, "name": "Northwind Traders", "address": {"city": "Lyon"}, "vat": "FR401"}
for match in finder.search(query).matches:
    print(match.right_id, round(match.score, 2), match.reasons)
# 1 0.94 ['name: trigrams', 'address.city: edit_distance', 'vat: exact']
# Record 2 has another VAT number, so it can never match.
```

Semantic fields load the default model on first use. Pass `Text(mode="semantic")` to use one ([models](https://github.com/leopoldch/univo/blob/main/docs/guide/models.md)).

## Documentation

- **Guide.** [Quickstart](https://github.com/leopoldch/univo/blob/main/docs/guide/quickstart.md), [models and offline use](https://github.com/leopoldch/univo/blob/main/docs/guide/models.md), [mutations](https://github.com/leopoldch/univo/blob/main/docs/guide/mutations.md), [snapshots](https://github.com/leopoldch/univo/blob/main/docs/guide/persistence.md), [concurrency](https://github.com/leopoldch/univo/blob/main/docs/guide/concurrency.md), [groups](https://github.com/leopoldch/univo/blob/main/docs/guide/groups.md), [embedding cache](https://github.com/leopoldch/univo/blob/main/docs/guide/embedding-cache.md), [approximate search](https://github.com/leopoldch/univo/blob/main/docs/guide/ann.md).
- **Reference.** [Errors](https://github.com/leopoldch/univo/blob/main/docs/reference/errors.md), [scoring](https://github.com/leopoldch/univo/blob/main/docs/reference/scoring.md), [duplicate definitions](https://github.com/leopoldch/univo/blob/main/docs/reference/duplicate-definitions.md), [stability](https://github.com/leopoldch/univo/blob/main/docs/reference/stability.md).
- **Design.** [Search](https://github.com/leopoldch/univo/blob/main/docs/design/search.md), [lexical candidates](https://github.com/leopoldch/univo/blob/main/docs/design/lexical-search.md), [vector search](https://github.com/leopoldch/univo/blob/main/docs/design/vector-search.md), [exact index](https://github.com/leopoldch/univo/blob/main/docs/design/exact-index.md).
- **Development.** [Setup](https://github.com/leopoldch/univo/blob/main/docs/development/setup.md), [performance](https://github.com/leopoldch/univo/blob/main/docs/development/performance.md), [dependencies](https://github.com/leopoldch/univo/blob/main/docs/development/dependencies.md), [releasing](https://github.com/leopoldch/univo/blob/main/docs/development/releasing.md).
