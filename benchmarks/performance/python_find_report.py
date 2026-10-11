"""Measure full lexical find with an early dense group and a sparse tail.

Use release wheels. Input generation is outside timing; each repeat constructs
an empty Finder and times conversion, indexing, retrieval and report creation.
"""

import argparse
import json
import platform
import random
import resource
import statistics
import sys
import time

from univo import Finder, Schema, Text


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--records", type=int, default=10_000)
    parser.add_argument("--limit", type=int, default=10_000)
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    rng = random.Random(15)
    names = ["duplicate"] * 150 + [
        "".join(rng.choices("abcdefghijklmnopqrstuvwxyz", k=20)) for _ in range(args.records - 150)
    ]
    records = [{"id": i, "name": name} for i, name in enumerate(names)]
    samples = []
    for _ in range(args.repeats):
        finder = Finder(schema=Schema(id_field="id", fields={"name": Text(mode="lexical")}))
        start = time.perf_counter_ns()
        report = finder.find(records, min_score=1, limit=args.limit)
        samples.append((time.perf_counter_ns() - start) / 1e6)
        assert report.total_matches == 150 * 149 // 2
        assert len(report.matches) == min(args.limit, report.total_matches)
        assert not report.candidate_truncated
    print(
        json.dumps(
            {
                "platform": platform.platform(),
                "python": platform.python_version(),
                "records": args.records,
                "early_group": 150,
                "limit": args.limit,
                "threads": 1,
                "partitions": 1,
                "model_bytes": 0,
                "seed": 15,
                "state": "new temporary index per repeat; no model; data generation excluded",
                "find_ms": samples,
                "median_ms": statistics.median(samples),
                "process_peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
                * (1 if sys.platform == "darwin" else 1024),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
