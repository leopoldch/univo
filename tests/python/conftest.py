import os
import pathlib
import tomllib
from typing import Any

import pytest

REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[2]


@pytest.fixture(scope="session")
def budgets() -> dict[str, Any]:
    path = pathlib.Path(
        os.environ.get("UNIVO_BUDGETS", REPOSITORY_ROOT / "benchmarks" / "budgets.toml")
    )
    with path.open("rb") as handle:
        return tomllib.load(handle)


@pytest.fixture(scope="session")
def time_slack() -> float:
    """Multiplier applied to time budgets on noisy shared runners."""
    slack = float(os.environ.get("UNIVO_PERF_SLACK", "1.0"))
    if slack < 1.0:
        raise ValueError("UNIVO_PERF_SLACK must be >= 1.0")
    return slack
