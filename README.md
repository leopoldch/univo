# Univo

**Compare anything. Keep every record clean.**

Fast, local duplicate detection for Python data: strings, records and nested objects.

[![Python 3.11+](https://img.shields.io/badge/python-3.11%2B-blue)](pyproject.toml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

Univo finds duplicates in any structured Python data: customers, products, organizations, configuration objects. You say which fields to compare and how: fuzzy text, meaning, exact values, nested paths. You also say which fields must agree and which records belong together. Univo then compares records field by field and explains every match.

A Rust engine does the work behind a small Python API. Inference is local: no record leaves your machine.

> **Status: 1.0 in preparation.** Not on PyPI yet. The [stability contract](docs/reference/stability.md) says what 1.0 will keep.

- **Fast.** 100,000 names are deduplicated in 4.8 s with embeddings and in 1.9 s lexically, on a laptop.
- **Complete.** Search returns every pair above the threshold. A work or output limit, when reached, is flagged in the report ([limits](docs/guide/quickstart.md#limits-and-reports)). Approximate search is opt-in.
- **Any structure.** Records are dicts, lists, strings, numbers, booleans and `None`, nested within configurable limits ([supported values](docs/guide/quickstart.md#records)). Dict keys compare in any order.
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

Univo's search is exact, while SemHash's index is approximate. Neither tool reaches the precision needed for automatic merging on names alone, so Univo reports scored candidates, not decisions. Method, limits and every result: [benchmarks](docs/benchmarks.md).

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

Semantic fields load the default model on first use. Pass `Text(mode="semantic")` to use one ([models](docs/guide/models.md)).

## Install

Wheels will be published on PyPI for Linux, macOS and Windows; Rust will not be needed to install them. Until then, build from source with Rust 1.89+, Python 3.11+ and [uv](https://docs.astral.sh/uv/):

```bash
git clone https://github.com/leopoldch/univo && cd univo
uv sync --group dev
```

## Documentation

- **Guide.** [Quickstart](docs/guide/quickstart.md), [models and offline use](docs/guide/models.md), [mutations](docs/guide/mutations.md), [snapshots](docs/guide/persistence.md), [concurrency](docs/guide/concurrency.md), [groups](docs/guide/groups.md), [embedding cache](docs/guide/embedding-cache.md), [approximate search](docs/guide/ann.md).
- **Reference.** [Errors](docs/reference/errors.md), [scoring](docs/reference/scoring.md), [duplicate definitions](docs/reference/duplicate-definitions.md), [stability](docs/reference/stability.md).
- **Design.** [Search](docs/design/search.md), [lexical candidates](docs/design/lexical-search.md), [vector search](docs/design/vector-search.md), [exact index](docs/design/exact-index.md).
- **Development.** [Setup](docs/development/setup.md), [performance](docs/development/performance.md), [dependencies](docs/development/dependencies.md), [releasing](docs/development/releasing.md).
