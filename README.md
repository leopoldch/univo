# Univo

**Fast, local duplicate detection for your data.**

Univo is a library under construction for finding duplicate candidates in strings and structured objects, with a reusable Rust core and a simple Python interface. Inference runs locally; applications do not need to manage embeddings or deploy a vector database.

## Project status

The library is under construction and not published yet. Rust provides types, schemas, normalization, errors, pairwise exact matching and an [exact index](docs/exact-index.md). Rust also provides [search](docs/search.md) by score across exact, lexical and semantic evidence. Python exposes [Finder](docs/quickstart.md), typed errors and [model download, cache and offline use](docs/models.md). See the [development guide](docs/development.md). API examples and performance budgets in the specifications describe targets unless explicitly validated.

## Quickstart

With a locally built wheel ([development](docs/development.md)):

```python
from univo import Finder

report = Finder(offline=True).find(["Amazon", "Amiazon", "Microsoft"])
for match in report.matches:
    print(match.left_id, match.right_id, match.score)
```

The Amazon/Amiazon pair is returned as a candidate. Default hybrid fields use lexical evidence and need no model. Explicit semantic fields load a local model lazily. Scores are not probabilities; [limits and structured examples](docs/quickstart.md).

## Specifications

- [Specification index and decision status](specs/README.md)
- [Product scope and release boundaries](specs/01-product.md)
- [Python API contracts](specs/02-python-api.md)
- [Matching and duplicate semantics](specs/03-matching.md)
- [Rust architecture and concurrency](specs/05-rust-architecture.md)
- [Performance and resource budgets](specs/07-performance.md)
- [Validation strategy](specs/08-validation.md)
- [Implementation roadmap and acceptance criteria](specs/10-roadmap.md)
- [Repository instructions](AGENTS.md)
- [Architecture decision records](docs/adr/README.md)
- [Duplicate definitions per profile](docs/duplicate-definitions.md)
- [Dependency register](docs/dependencies.md)
- [Error catalog](docs/errors.md)
- [Models, cache and offline use](docs/models.md)
- [Pairwise scoring](docs/scoring.md)
- [Vector search](docs/vector-search.md)
- [Lexical candidates](docs/lexical-search.md)
- [Search](docs/search.md)
- [Concurrency and check_or_add](docs/concurrency.md)
- [Groups](docs/groups.md)
- [Comparison with SemHash](benchmarks/comparison/README.md)
- [Releasing](docs/releasing.md)

## Design priorities

- Fast search and bounded memory, verified with reproducible benchmarks.
- Local inference with a cached model and strict offline operation.
- Exact, lexical and semantic evidence with explicit conflicts and explanations.
- Minimal dependencies and precompiled Python wheels on supported platforms.
- Explicit uncertainty: similarity is not identity, and no records are deleted or merged implicitly.

The model, approximate-search backend and default thresholds will be selected through the experiments in the roadmap. Java bindings and other integrations follow validation of the Rust/Python core.

## License

MIT, see [LICENSE](LICENSE).
