"""Compare full search with repeated selected text, cache disabled/enabled.

A fresh Finder per repeat; local model and one thread. One target per partition
isolates conversion/inference from large-partition retrieval costs. This is a
reuse workload, not a general search latency or cold-disk benchmark.
"""

import argparse
import json
import platform
import resource
import statistics
import sys
import time
from pathlib import Path

from univo import Finder, Schema, Text


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--records", type=int, default=1000)
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    sentence = (
        "local search compares records and descriptions with exact values and semantic evidence "
        "while preserving partitions and missing fields for efficient duplicate detection "
        "without sending any content to a remote service"
    )
    records = [{"id": i, "tenant": i, "text": f"{sentence} item {i}"} for i in range(args.records)]
    schema = Schema(id_field="id", partition_by=["tenant"], fields={"text": Text(mode="semantic")})
    result: dict[str, object] = {}
    for budget in (0, 4 << 20):
        additions, p95, p99 = [], [], []
        for _ in range(args.repeats):
            finder = Finder(
                schema=schema,
                model_path=args.model,
                offline=True,
                threads=1,
                embedding_cache_bytes=budget,
            )
            start = time.perf_counter_ns()
            finder.add(records)
            additions.append((time.perf_counter_ns() - start) / 1e6)
            for record in records[:20]:
                finder.search({**record, "id": record["id"] + args.records})
            samples = []
            for record in records:
                query = {**record, "id": record["id"] + args.records, "ignored": "new metadata"}
                start = time.perf_counter_ns()
                report = finder.search(query)
                samples.append((time.perf_counter_ns() - start) / 1e6)
                assert len(report.matches) == 1
                assert report.matches[0].right_id == record["id"]
                assert not report.candidate_truncated
            p95.append(statistics.quantiles(samples, n=20, method="inclusive")[-1])
            p99.append(statistics.quantiles(samples, n=100, method="inclusive")[-1])
        result[str(budget)] = {
            "add_including_local_model_load_ms": additions,
            "search_p95_ms": p95,
            "search_p99_ms": p99,
            "stats_last_repeat": finder.stats(),
        }
    result["metadata"] = {
        "platform": platform.platform(),
        "python": platform.python_version(),
        "threads": 1,
        "records": args.records,
        "partitions": args.records,
        "queries_per_repeat": args.records,
        "repeats": args.repeats,
        "model_files_bytes": sum(
            (args.model / name).stat().st_size
            for name in ("config.json", "tokenizer.json", "model.safetensors")
        ),
        "model_revision": args.model.name,
        "model_state": "local files in OS cache; fresh Finder per repeat",
        "query_state": "20 warmups; same selected text, new ID/unselected metadata; no truncation",
        "process_peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        * (1 if sys.platform == "darwin" else 1024),
    }
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
