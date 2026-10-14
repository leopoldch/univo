# Univo

**Fast, local duplicate detection for your data.**

Univo finds duplicate candidates among strings and records, from exact, lexical and semantic evidence. A Rust engine does the work behind a small Python API; nothing leaves your machine.

> **Pre-alpha.** Not on PyPI yet; APIs and formats may change.

- **Fast.** 100,000 names deduplicated in 4.8 s semantically (SemHash: 8.7 s) and in about 2 s lexically, on a laptop. [Measurements](docs/development/performance.md).
- **Exact.** Search finds every pair above the threshold. Approximate search is opt-in.
- **Local.** A 15 MB multilingual model, verified and cached; strict offline mode.
- **Explained.** Every match comes with its per-field evidence. Records are never merged or deleted for you.

## Quickstart

```python
from univo import Finder

report = Finder(offline=True).find(["Amazon", "Amiazon", "Microsoft"])
for match in report.matches:
    print(match.left_id, match.right_id, match.score)
```

Records, with IDs, partitions and vetoes, are covered in the [guide](docs/guide/quickstart.md).

## Install from source

Requires Rust 1.89+, Python 3.11+ and [uv](https://docs.astral.sh/uv/).

```bash
git clone https://github.com/leopoldch/univo && cd univo
uv sync --group dev
```

## Learn more

- [Documentation](docs/README.md)
- [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [Changelog](CHANGELOG.md)

MIT [license](LICENSE).
