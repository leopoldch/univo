# Tools

Maintenance scripts, standard library only unless noted. Run them from the repository root; tests import them through the `pytest` `pythonpath` of `pyproject.toml`, and their tests live in [`tests/tools`](../tests/tools).

| Path | Purpose |
| --- | --- |
| [`build_default_model.py`](build_default_model.py) | Rebuilds the default model byte for byte from its pinned upstream files. [`default_model/`](default_model) holds its model card and license. The published model card links to this path, so it stays at the root of `tools/`. |
| [`checks/check_links.py`](checks/check_links.py) | Fails on Markdown links to repository paths that do not exist. |
| [`checks/check_native_dependencies.py`](checks/check_native_dependencies.py) | Fails when a network crate is linked into a distributed Rust artifact. |
| [`checks/check_release.py`](checks/check_release.py) | Release checks run by the release workflow before publishing. |
| [`checks/check_wheel.py`](checks/check_wheel.py) | Checks a built wheel against its size budget and file layout. |
| [`fixtures/build_critical_corpus.py`](fixtures/build_critical_corpus.py) | Builds the critical corpus of `tests/fixtures/critical` from its reviewed case table; `--check` fails when it is stale. |
| [`fixtures/build_tiny_model.py`](fixtures/build_tiny_model.py) | Builds the tiny test model of `tests/fixtures/models/tiny`; `--check` fails when it is stale. |
| [`fixtures/build_unicode_fixtures.py`](fixtures/build_unicode_fixtures.py) | Generates the CPython Unicode oracles of the Rust normalizers. |
| [`quality/build_quality_corpus.py`](quality/build_quality_corpus.py) | Builds the entity-disjoint name benchmark from the pinned ROR dump. |
| [`quality/evaluate_quality.py`](quality/evaluate_quality.py) | Evaluates the installed package on that benchmark: calibrates on validation, evaluates once on test. |
| [`quality/quality_metrics.py`](quality/quality_metrics.py), [`quality/quality_profiles.py`](quality/quality_profiles.py) | Metrics and profile regressions used by the evaluation. |

The protocols and results are in [`benchmarks/quality`](../benchmarks/quality/README.md).
