"""Latency of checking one new record against an index of 100,000.

The synthetic company names of scale.py, one partition, the default
threshold of 0.8: 100,000 names are added, then 500 new ones go through
`check_or_add` one at a time, each timed on its own. Prints the median and
the p95, lexically and, with --model-path, with embeddings.
"""

from __future__ import annotations

import argparse
import statistics
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from scale import names
from univo import Finder, Schema, Text

RECORDS = 100_000
QUERIES = 500


def measure(label: str, finder: Finder, data: list[str]) -> None:
    started = time.perf_counter()
    finder.add([{"id": i, "name": name} for i, name in enumerate(data[:RECORDS])])
    added = time.perf_counter() - started
    times = []
    for i, name in enumerate(data[RECORDS:]):
        started = time.perf_counter()
        finder.check_or_add({"id": RECORDS + i, "name": name})
        times.append(time.perf_counter() - started)
    times.sort()
    p95 = times[int(len(times) * 0.95)]
    print(
        f"{label}: add {RECORDS:,} in {added:.2f} s; check_or_add median "
        f"{statistics.median(times) * 1e3:.2f} ms, p95 {p95 * 1e3:.2f} ms"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--model-path", help="local default model, for the semantic run")
    arguments = parser.parse_args()
    data = names(RECORDS + QUERIES)
    capacity = RECORDS + QUERIES
    lexical = Schema(id_field="id", fields={"name": Text(mode="lexical")})
    measure("lexical", Finder(schema=lexical, offline=True, max_records=capacity), data)
    if arguments.model_path:
        semantic = Schema(id_field="id", fields={"name": Text(mode="semantic")})
        finder = Finder(
            schema=semantic, model_path=arguments.model_path, offline=True, max_records=capacity
        )
        measure("semantic", finder, data)


if __name__ == "__main__":
    main()
