"""Measure release-wheel mutations, including amortized compaction spikes.

Run each repetition in a fresh process. One thread, local warm model files,
cache disabled, selected records generated outside mutation timers. Peak RSS
covers the whole process; it is not incremental engine memory.
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
from pathlib import Path
from typing import Any

from univo import Finder, Schema, Text


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--records", type=int, default=100_000)
    parser.add_argument("--churn", type=int, default=60)
    args = parser.parse_args()
    rng = random.Random(15)
    sentence = (
        "local search compares records and descriptions with exact values and semantic evidence "
        "while preserving partitions and missing fields for efficient duplicate detection "
        "without sending any content to a remote service"
    )
    records: list[dict[str, Any]] = [
        {
            "id": i,
            "tenant": i % 100,
            "name": "".join(rng.choices("abcdefghijklmnopqrstuvwxyz", k=20)),
            "description": f"{sentence} item {i}",
        }
        for i in range(args.records)
    ]
    size = min(1000, args.records // 10)
    schema = Schema(
        id_field="id",
        partition_by=["tenant"],
        fields={
            "name": Text(mode="lexical"),
            "description": Text(mode="semantic"),
        },
    )
    finder = Finder(
        schema=schema,
        model_path=args.model,
        offline=True,
        threads=1,
        max_records=args.records,
        embedding_cache_bytes=0,
    )
    timings = {}
    start = time.perf_counter_ns()
    finder.add(records)
    timings["add_including_model_load_ms"] = (time.perf_counter_ns() - start) / 1e6
    lexical = [{**record, "name": "changed " + record["name"]} for record in records[:size]]
    semantic = [
        {**record, "description": record["description"] + " world"}
        for record in records[size : 2 * size]
    ]
    for label, batch in [
        ("upsert_lexical_only_ms", lexical),
        ("upsert_semantic_changed_ms", semantic),
    ]:
        start = time.perf_counter_ns()
        assert finder.upsert(batch) == [record["id"] for record in batch]
        timings[label] = (time.perf_counter_ns() - start) / 1e6
    removed = list(range(args.records - size, args.records))
    start = time.perf_counter_ns()
    assert finder.remove(removed) == []
    timings["remove_ms"] = (time.perf_counter_ns() - start) / 1e6
    churn = []
    compactions = 0
    retained = []
    for _ in range(args.churn):
        before = finder.stats()["retained_slots"]
        start = time.perf_counter_ns()
        finder.upsert(lexical)
        churn.append((time.perf_counter_ns() - start) / 1e6)
        after = finder.stats()["retained_slots"]
        compactions += int(after < before)
        retained.append(after)
        assert after <= 2 * args.records
    assert finder.stats()["records"] == args.records - size
    for record in lexical[:10]:
        result = finder.search({**record, "id": args.records + record["id"]}, min_score=0.99)
        assert any(match.right_id == record["id"] for match in result.matches)
    print(
        json.dumps(
            {
                "timings_ms": timings,
                "unchanged_churn_samples_ms": churn,
                "churn_median_ms": statistics.median(churn),
                "churn_max_ms": max(churn),
                "compactions": compactions,
                "retained_slots": retained,
                "stats": finder.stats(),
                "metadata": {
                    "platform": platform.platform(),
                    "python": platform.python_version(),
                    "rust": subprocess.check_output(["rustc", "--version"], text=True).strip(),
                    "cpu_and_ram": subprocess.check_output(
                        ["sysctl", "-n", "machdep.cpu.brand_string", "hw.memsize"], text=True
                    ).splitlines()
                    if sys.platform == "darwin"
                    else platform.processor(),
                    "threads": 1,
                    "records": args.records,
                    "batch_size": size,
                    "partitions": 100,
                    "model_revision": args.model.name,
                    "model_bytes": sum(
                        (args.model / name).stat().st_size
                        for name in ("config.json", "model.safetensors", "tokenizer.json")
                    ),
                    "state": "fresh process/index, warm local model files; cache disabled",
                    "rss_scope": "whole process: inputs, model, indexes, vectors, compaction",
                    "process_peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
                    * (1 if sys.platform == "darwin" else 1024),
                },
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
