# Univo

**Fast, local duplicate detection for your data.**

Univo is a planned library for finding duplicate candidates in strings and structured objects, with a reusable Rust core and a simple Python interface. Inference runs locally; applications do not need to manage embeddings or deploy a vector database.

## Project status

The library is under construction and not published yet. The repository contains the specifications and the Rust/Python build skeleton ([development guide](docs/development.md)). API examples and performance budgets in the specifications describe the intended implementation, not existing functionality or measured results.

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

## Design priorities

- Fast search and bounded memory, verified with reproducible benchmarks.
- Local inference with a cached model and strict offline operation.
- Exact, lexical and semantic evidence with explicit conflicts and explanations.
- Minimal dependencies and precompiled Python wheels on supported platforms.
- Explicit uncertainty: similarity is not identity, and no records are deleted or merged implicitly.

The model, approximate-search backend and default thresholds will be selected through the experiments in the roadmap. Java bindings and other integrations follow validation of the Rust/Python core.

## License

MIT, see [LICENSE](LICENSE).
