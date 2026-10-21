# Contributing

## Setup

Requires Rust 1.89+, Python 3.11+, [uv](https://docs.astral.sh/uv/) and [just](https://just.systems).

```bash
just setup   # create .venv and build the extension
just check   # local checks and tests
```

CI also builds and tests the wheels on every platform, and runs the tests that need the default model, which are skipped locally unless `UNIVO_TEST_MODEL_DIR` is set.

`just` lists the other tasks. Details: [development setup](docs/development/setup.md).

## Rules

- **One subject per pull request**, with its tests and docs.
- **Contracts first.** The [specs](specs/01-product.md) (in French) and the [decision records](docs/adr) set the behavior. Update them with any change of API, format or model compatibility.
- **Decisions are recorded.** A choice of model, backend, threshold or format gets a new ADR, numbered after the last one: context, decision, alternatives, evidence, consequences, and when to revisit it. An accepted ADR is never rewritten; a later one supersedes it and updates its status line.
- **Logic stays in Rust.** Python only converts inputs and outputs.
- **Performance claims are measured.** Use release builds, compare against `main` in ABBA order, and show that results are identical ([policy](docs/development/performance.md)).
- **New dependencies** go in the [register](docs/development/dependencies.md).
- **English** for code, comments and docs. Comments explain why, briefly.
- **Commits** start with a type (`feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `chore`, `ci`) and say what changed and how it was validated.

Automated agents also follow [AGENTS.md](AGENTS.md).
