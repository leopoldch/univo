# Documentation

## Guide

Using Univo from Python.

- [Quickstart](guide/quickstart.md): `Finder`, schemas, `find`, `search`, reports and their limits.
- [Models, cache and offline use](guide/models.md): the default model, other models, downloads and strict offline operation.
- [Mutations by ID](guide/mutations.md): `upsert` and `remove`, generations and compaction.
- [Snapshots](guide/persistence.md): `save` and `load`, atomic publication and compatibility.
- [Concurrency and check_or_add](guide/concurrency.md): one Finder across threads, and atomic insertion of non-duplicates.
- [Groups](guide/groups.md): conservative groups built from pairs.
- [Embedding cache](guide/embedding-cache.md): how embeddings are reused and bounded.
- [Approximate semantic search](guide/ann.md): the explicit HNSW profile.

## Reference

- [Errors](reference/errors.md): every error code, with its message and hint.
- [Pairwise scoring](reference/scoring.md): how two records are compared into a score, a coverage and contradictions.
- [Duplicate definitions](reference/duplicate-definitions.md): what counts as a duplicate, per profile.

## Design

How the Rust engine finds candidates.

- [Search](design/search.md): one candidate lane per field, their union, and the exhaustive oracle.
- [Lexical candidates](design/lexical-search.md): the trigram index and its exact bounds.
- [Vector search](design/vector-search.md): exhaustive search over embeddings.
- [Exact index](design/exact-index.md): equality after normalization.
- [Architecture decision records](adr/README.md): each choice, its alternatives and its evidence.

## Development

- [Development setup](development/setup.md): building, testing and checking.
- [Performance](development/performance.md): the measurement policy and every measurement.
- [Dependency register](development/dependencies.md): each dependency, its features, license and footprint.
- [Releasing](development/releasing.md): publishing to PyPI from a GitHub release.

The [specifications](../specs/README.md), in French, set the product's contracts and roadmap.
