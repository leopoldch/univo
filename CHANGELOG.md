# Changelog

All notable changes are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions will follow [Semantic Versioning](https://semver.org/) from the first release.

## [Unreleased]

### Added

- **Python API.**
  - `Finder` with `find`, `search`, `add`, `upsert`, `remove`, `group`, `check_or_add`, `save` and `load`.
  - Schemas with typed IDs, partitions, lexical, semantic and hybrid text fields, and exact fields with vetoes.
  - Typed errors with stable codes, paths and hints ([catalog](docs/reference/errors.md)).
- **Matching.**
  - Pairwise scoring over exact, lexical (edit distance and trigrams), semantic and acronym evidence, with per-field explanations and coverage.
  - Names with distinct words ("Institute of Epidemiology" / "Institute of Oncology") score lower and report a conflict (scoring version 4).
  - Unicode NFC and case folding for text, and identity or hostname normalization for exact fields.
- **Search.**
  - One complete candidate lane per field, verified against an exhaustive oracle.
  - Opt-in partitioned HNSW retrieval that reports its approximation.
  - Parallel all-pairs `find` and `group` on a per-Finder worker pool.
- **Models.**
  - A local multilingual static default model, int8 with 128 dimensions (15 MB), pinned by revision and checksum, downloaded once and cached.
  - Strict offline mode, and a bounded embedding cache.
- **Persistence and concurrency.**
  - Versioned snapshots with a single atomic publication point.
  - Parallel searches and an atomic local `check_or_add`.
- **Release.** PyPI publication from GitHub releases with trusted publishing.

### Performance

On 100,000 names, on an Apple M2:
- **Semantic all-pairs deduplication:** 4.8 s, exact. SemHash takes 8.7 s with the same model and an approximate index.
- **Lexical all-pairs deduplication:** about 2 s.

Lexical candidates are rejected early by exact trigram-signature and character-count bounds.

See [performance](docs/development/performance.md) for the method and every measurement.
