# Tools

Maintenance scripts, run from the repository root. Their tests are in [`tests/tools`](../tests/tools).

| Folder | Scripts |
| --- | --- |
| [`checks/`](checks) | Links, wheel, release and native-dependency checks run by CI |
| [`fixtures/`](fixtures) | Generators of the test fixtures; `--check` fails when one is stale |
| [`quality/`](quality) | The [quality benchmark](../benchmarks/quality/README.md) pipeline |

[`build_default_model.py`](build_default_model.py) rebuilds the default model; it stays here because the published model card links to this path.
