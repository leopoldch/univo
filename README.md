# Univo

**Fast, local duplicate detection for your data.**

Univo finds duplicate candidates among strings and structured records. It combines exact, lexical and semantic evidence, explains every match, and runs entirely on your machine: a Rust engine behind a small Python API, with no vector database to deploy and no data sent anywhere.

> **Status: pre-alpha.** Univo is not published on PyPI yet. Build it from source (below); APIs and formats may still change.

## Highlights

- **Fast.**
  - **Semantic deduplication** of 100,000 names takes 4.8 s on a laptop, exactly: every pair above the threshold is found. SemHash takes 8.7 s with the same model and an approximate index.
  - **Lexical deduplication** of 100,000 names takes about 2 s. [Measurements](docs/development/performance.md).
- **Exact by default.** Candidate search is complete for the scoring rule. Approximate retrieval (HNSW) is an explicit option, and reports say when it was used.
- **Local and offline.** Inference runs locally with a small static model (15 MB, multilingual), downloaded once, verified against pinned checksums and cached. Strict offline mode never touches the network.
- **Explainable.** Each match carries per-field similarities, the signal behind each one, and its coverage. Scores rank candidates; they are not probabilities.
- **Built for records.** Schemas with typed IDs, partitions such as tenants, exact fields with vetoes, and missing values that stay missing evidence.
- **Safe operations.**
  - `upsert` and `remove` by ID;
  - atomic `check_or_add`;
  - versioned snapshots with a single publication point.

  Records are never merged or deleted implicitly.

## Quickstart

```python
from univo import Finder

report = Finder(offline=True).find(["Amazon", "Amiazon", "Microsoft"])
for match in report.matches:
    print(match.left_id, match.right_id, match.score)
```

Structured records, with an ID, a partition and an exact field that vetoes conflicting matches:

```python
from univo import Exact, Finder, Schema, Text

schema = Schema(
    id_field="id",
    partition_by=["tenant"],
    fields={"name": Text(), "environment": Exact(conflict="reject")},
)
finder = Finder(schema=schema, offline=True)
records = [
    {"id": "org-1", "tenant": "shop", "name": "Amazon", "environment": "prod"},
    {"id": "org-2", "tenant": "shop", "name": "AMAZON", "environment": "prod"},
    {"id": "org-3", "tenant": "shop", "name": "Amazon", "environment": "test"},
]
finder.add(records)
report = finder.search(records[0])
assert [match.right_id for match in report.matches] == ["org-2"]
```

Text fields are compared lexically by default and need no model. `Text(mode="semantic")` uses embeddings: the model loads lazily on first use. The [quickstart guide](docs/guide/quickstart.md) covers options, limits and reports.

## Installation

Univo needs Rust (1.89 or later), Python 3.11 or later and [uv](https://docs.astral.sh/uv/):

```bash
git clone https://github.com/leopoldch/univo
cd univo
uv sync --group dev        # creates .venv and builds the extension in release mode
uv run python -c "import univo; print(univo.__version__)"
```

Published releases will ship precompiled wheels: installing them will not need Rust.

## Documentation

- [Guide](docs/README.md#guide): models and offline use, mutations, snapshots, concurrency, groups.
- [Reference](docs/README.md#reference): errors, pairwise scoring, duplicate definitions.
- [Design](docs/README.md#design): how search finds candidates, and the [architecture decisions](docs/adr/README.md).
- [Performance](docs/development/performance.md) and the [comparison with SemHash](benchmarks/comparison/README.md).

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/univo-core` | Values, schemas, normalization, record preparation and pairwise scoring |
| `crates/univo-embed` | Model references, pinned registry, verified cache and static inference |
| `crates/univo-engine` | Indexes, candidate search, reports, groups and snapshots |
| `crates/univo-python` | The PyO3 extension: conversions only |
| `python/univo` | The Python package |
| `docs`, `specs` | Documentation, and the product specifications (in French) |
| `benchmarks`, `tests`, `tools` | Measurements, tests and fixtures, and maintenance scripts |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow and the [development setup](docs/development/setup.md). Report vulnerabilities privately, as described in [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE).
