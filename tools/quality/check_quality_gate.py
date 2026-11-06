"""Fail when validation recall at the declared precision falls below its baseline.

Compares counts, not rates, so the gate has no float tolerance to tune. A pull request
that lowers recall on purpose updates the baseline file, which makes the loss visible in
its diff.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


def regressions(result: dict[str, Any], baseline: dict[str, Any]) -> list[str]:
    if result["dataset_sha256"] != baseline["dataset_sha256"]:
        return ["the corpus differs from the baseline's: results are not comparable"]
    if result["threshold"] is None:
        return [
            f"no threshold reaches {result['minimum_validation_precision']:.0%} precision "
            f"with at least {result['minimum_validation_tp']} true positives on validation"
        ]
    found, expected = result["validation"]["tp"], baseline["validation"]["tp"]
    if found < expected:
        total = baseline["validation"]["positives"]
        return [f"validation recall fell: {found}/{total} duplicates found, baseline {expected}"]
    return []


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result", type=Path)
    parser.add_argument("baseline", type=Path)
    args = parser.parse_args()
    result = json.loads(args.result.read_text(encoding="utf-8"))
    baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    failures = regressions(result, baseline)
    for failure in failures:
        print(f"{args.baseline.name}: {failure}", file=sys.stderr)
    if failures:
        raise SystemExit(1)
    print(
        f"{args.baseline.name}: {result['validation']['tp']} duplicates found on validation, "
        f"baseline {baseline['validation']['tp']}"
    )


if __name__ == "__main__":
    main()
