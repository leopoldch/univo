import statistics
from typing import Any

import pytest

from .._subprocess import run_isolated

IMPORT_TIME_MS = """
import json, time
start = time.perf_counter_ns()
import univo
print(json.dumps((time.perf_counter_ns() - start) / 1e6))
"""


@pytest.mark.perf
def test_import_p95_within_budget(budgets: dict[str, Any], time_slack: float) -> None:
    budget = budgets["python"]["import"]
    samples = [run_isolated(IMPORT_TIME_MS) for _ in range(budget["samples"])]

    p95 = statistics.quantiles(samples, n=20, method="inclusive")[-1]
    limit = budget["p95_ms"] * time_slack
    assert p95 <= limit, f"import p95 {p95:.2f} ms exceeds {limit:.2f} ms"
