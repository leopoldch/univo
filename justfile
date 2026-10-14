# Development tasks. Run `just` to list them; each recipe is the command CI runs.

set shell := ["bash", "-euo", "pipefail", "-c"]

# List the tasks.
default:
    @just --list

# Create .venv and build the extension in release mode.
setup:
    uv sync --group dev

# Rebuild the Python extension after Rust changes.
develop:
    uv run maturin develop --release

# Format Rust and Python code.
fmt:
    cargo fmt --all
    uv run ruff format .

# Every static check: formatting, lints, types, links, native dependencies and generated fixtures.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    uv run ruff check .
    uv run ruff format --check .
    uv run mypy
    uv run python tools/checks/check_links.py .
    uv run python tools/checks/check_native_dependencies.py univo-python univo-engine univo-embed
    uv run python tools/fixtures/build_critical_corpus.py --check
    uv run python tools/fixtures/build_tiny_model.py --check

# Rust tests, in debug then in release, where the resource budgets run.
test-rust:
    cargo test --workspace --locked
    cargo test --workspace --locked --release

# Python tests against a fresh build of the extension.
test-python: develop
    uv run pytest

# Every test.
test: test-rust test-python

# Every check CI runs before a pull request.
check: lint test

# Build a release wheel into dist/ and check its size and layout.
wheel:
    uv run maturin build --release --out dist
    uv run python tools/checks/check_wheel.py dist/*.whl

# Regenerate docs/reference/errors.md from the error catalog.
errors-doc:
    UNIVO_UPDATE_DOCS=1 cargo test -p univo-core --test error_catalog

# Core micro-benchmarks.
bench:
    cargo bench -p univo-core
