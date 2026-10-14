# Contributing

Thank you for helping with Univo. This page covers the workflow; the [development setup](docs/development/setup.md) covers the tools and every check in detail.

## Setup

You need Rust (1.89 or later), Python 3.11 or later, [uv](https://docs.astral.sh/uv/) and, for the shortcuts below, [just](https://just.systems).

```bash
just setup     # create .venv and build the extension in release mode
just check     # every check CI runs: formatting, lints, links, fixtures, Rust and Python tests
```

`just` lists every task. Each one is a short recipe in the [justfile](justfile), so the underlying commands can also be run by hand.

## Making a change

- **One change per pull request.** Keep each one to a single subject: a feature, a fix, a refactor, a measurement. A large plan becomes several pull requests.
- **Read the contract first.** The [specifications](specs/README.md) and [architecture decisions](docs/adr/README.md) set the behavior. When a change alters an API, an on-disk format or model compatibility, update the contract and the documentation with it. When it settles an open choice, record an ADR with its alternatives and evidence.
- **Keep the logic in Rust.** Matching, indexing and decisions live in the Rust crates. The Python package converts inputs and outputs and never scores.
- **Test what the change risks.** Add tests that fail without the change; for subtle invariants, check that they catch a deliberate mutation of the code. Prefer the exhaustive oracle as the reference for any search change.
- **Measure performance changes.**
  - **Builds.** Use release builds, and compare against `main` on the same machine, alternating the two (ABBA order).
  - **Results.** Check that results are identical.
  - **Report.** Give the hardware, data, threads and conditions with the numbers. The policy is in [performance](docs/development/performance.md).
- **Dependencies.** Record each new one in the [dependency register](docs/development/dependencies.md), with its purpose, features, license and footprint.

## Style

- **Language.** Code, comments and documentation are in English; the specifications are in French.
- **Rust.** Workspace lints apply (`clippy::pedantic`, no `unsafe`). Errors are typed, and no panic crosses the Python boundary.
- **Python.** Ruff and mypy in strict mode apply.
- **Comments** explain why, not what, and stay short.

## Commits and pull requests

- **Commit messages.** They start with a type (`feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `chore`, `ci`) and say what changed and how it was validated.
- **Pull request descriptions** list the validation and any remaining limitation. The [template](.github/PULL_REQUEST_TEMPLATE.md) gives the outline.
- **Data.** Keep user data, credentials and local artifacts out of commits.

Automated agents also follow [AGENTS.md](AGENTS.md).
