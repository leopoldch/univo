"""Release-wheel snapshot round trips on synthetic standard-workload records.

Run each size in a fresh process. Five independent save/load cycles, OS-cache
warm local files, one thread. Load includes model verification/loading and lane
reconstruction; no inference for stored vectors. No disk-cold latency claim.
"""

import argparse
import gc
import json
import platform
import random
import resource
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from univo import Finder, Schema, Text


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--records", type=int, default=1000)
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    rng = random.Random(14)
    sentence = (
        "local search compares records and descriptions with exact values and semantic evidence "
        "while preserving partitions and missing fields for efficient duplicate detection "
        "without sending any content to a remote service"
    )
    records = [
        {
            "id": i,
            "tenant": i % 100,
            "name": "".join(rng.choices("abcdefghijklmnopqrstuvwxyz", k=20)),
            "description": f"{sentence} item {i}",
        }
        for i in range(args.records)
    ]
    queries = [{**record, "id": args.records + record["id"]} for record in records[:10]]
    schema = Schema(
        id_field="id",
        partition_by=["tenant"],
        fields={
            "name": Text(mode="lexical"),
            "description": Text(mode="semantic"),
        },
    )
    samples: dict[str, list[float]] = {
        "add_including_local_model_load_ms": [],
        "save_ms": [],
        "load_ms": [],
    }
    phase_peaks: list[dict[str, int]] = []
    rss_unit = 1 if sys.platform == "darwin" else 1024
    snapshot_bytes = 0
    dimension = 0
    for _ in range(args.repeats):
        with tempfile.TemporaryDirectory(prefix="univo-snapshot-bench-") as temporary:
            path = Path(temporary) / "index"
            finder = Finder(
                schema=schema,
                model_path=args.model,
                offline=True,
                threads=1,
                max_records=args.records,
            )
            start = time.perf_counter_ns()
            finder.add(records)
            samples["add_including_local_model_load_ms"].append(
                (time.perf_counter_ns() - start) / 1e6
            )
            phase = {"after_add": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * rss_unit}
            before = [finder.search(query) for query in queries]
            start = time.perf_counter_ns()
            finder.save(path)
            samples["save_ms"].append((time.perf_counter_ns() - start) / 1e6)
            generation = path / (path / "CURRENT").read_text().splitlines()[0]
            snapshot_bytes = sum(file.stat().st_size for file in generation.iterdir())
            dimension = json.loads((generation / "manifest.json").read_text())["dimension"]
            del finder
            gc.collect()
            start = time.perf_counter_ns()
            loaded = Finder.load(path, offline=True, threads=1, max_records=args.records)
            samples["load_ms"].append((time.perf_counter_ns() - start) / 1e6)
            phase["after_load"] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * rss_unit
            phase_peaks.append(phase)
            assert loaded.stats()["records"] == args.records
            assert loaded.stats()["generation"] == 1
            assert loaded.stats()["embedding_cache_entries"] == 0
            assert [loaded.search(query) for query in queries] == before
            stats = loaded.stats()
            del loaded
            gc.collect()
    output = {
        "samples": samples,
        "cumulative_peak_rss_by_phase_bytes": phase_peaks,
        "median_ms": {key: statistics.median(values) for key, values in samples.items()},
        "snapshot_generation_bytes": snapshot_bytes,
        "last_stats_after_queries": stats,
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
            "partitions": min(args.records, 100),
            "repeats": args.repeats,
            "name_length": 20,
            "description_words": len(sentence.split()) + 2,
            "model_revision": args.model.name,
            "dimension": dimension,
            "model_bytes": sum(
                (args.model / name).stat().st_size
                for name in ("config.json", "model.safetensors", "tokenizer.json")
            ),
            "state": "warm OS cache, fresh Finder per cycle; original released before load",
            "process_peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
            * (1 if sys.platform == "darwin" else 1024),
            "rss_scope": "entire process: generated Python records, model, lanes and vectors",
            "storage": "local filesystem, system temporary directory; no network or cold-disk test",
        },
    }
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
