"""Measure installed Finder end to end, without a model or network.

Run with a release wheel, e.g. python python_finder.py --records 100000.
Synthetic fixed-seed records, one lexical field, one thread. Data generation
is outside add/search timers. New query IDs avoid self-exclusion; queries
change one character so conversion and normalization cannot be cached.
"""

import argparse
import json
import platform
import random
import resource
import statistics
import subprocess
import sys
import time

from univo import Finder, Schema, Text, __version__


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--records", type=int, default=100_000)
    parser.add_argument("--queries", type=int, default=1000)
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    rng = random.Random(11)
    names = ["".join(rng.choices("abcdefghijklmnopqrstuvwxyz", k=20)) for _ in range(args.records)]
    records = [{"id": i, "name": name} for i, name in enumerate(names)]
    queries = [{"id": args.records + i, "name": "!" + names[i][1:]} for i in range(args.queries)]
    add_ms = []
    p95_ms = []
    p99_ms = []
    for _ in range(args.repeats):
        finder = Finder(
            schema=Schema(id_field="id", fields={"name": Text(mode="lexical")}),
            max_records=args.records,
        )
        start = time.perf_counter_ns()
        finder.add(records)
        add_ms.append((time.perf_counter_ns() - start) / 1e6)
        for query in queries[:20]:
            finder.search(query, min_score=0.8, limit=10)
        samples = []
        for query in queries:
            start = time.perf_counter_ns()
            result = finder.search(query, min_score=0.8, limit=10)
            samples.append((time.perf_counter_ns() - start) / 1e6)
            assert not result.candidate_truncated
            assert any(match.right_id == query["id"] - args.records for match in result.matches)
        p95_ms.append(statistics.quantiles(samples, n=20, method="inclusive")[-1])
        p99_ms.append(statistics.quantiles(samples, n=100, method="inclusive")[-1])
    cold = []
    construction = []
    for _ in range(30):
        code = (
            "import json,time; t=time.perf_counter_ns(); import univo; "
            "a=(time.perf_counter_ns()-t)/1e6; from univo import Finder; "
            "t=time.perf_counter_ns(); f=Finder(); "
            "print(json.dumps([a,(time.perf_counter_ns()-t)/1e6]))"
        )
        values = json.loads(subprocess.check_output([sys.executable, "-I", "-c", code], text=True))
        cold.append(values[0])
        construction.append(values[1])
    print(
        json.dumps(
            {
                "platform": platform.platform(),
                "processor": platform.processor(),
                "python": platform.python_version(),
                "univo": __version__,
                "records": args.records,
                "queries_per_repeat": args.queries,
                "repeats": args.repeats,
                "threads": 1,
                "partitions": 1,
                "model_bytes": 0,
                "model_state": "not loaded; lexical schema",
                "query_state": "warm index, new query IDs and texts, no embedding cache",
                "add_ms": add_ms,
                "search_p95_ms": p95_ms,
                "search_p99_ms": p99_ms,
                "import_p95_ms": statistics.quantiles(cold, n=20, method="inclusive")[-1],
                "construction_p95_ms": statistics.quantiles(construction, n=20, method="inclusive")[
                    -1
                ],
                "index_bytes": finder.stats()["index_bytes"],
                "process_peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
                * (1 if sys.platform == "darwin" else 1024),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
