"""Release-wheel ANN measurements; run repetitions in fresh processes.

Synthetic one-positive queries, fixed seed and threshold. Whole-process RSS
includes Python inputs, local model, indexes and snapshot reconstruction.
This is a performance fixture, not a calibrated business quality corpus.
"""

import argparse
import json
import platform
import random
import resource
import statistics
import tempfile
import time
from pathlib import Path
from typing import Any, Literal

from univo import Finder, Schema, Text


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model", type=Path)
    parser.add_argument("--records", type=int, default=100_000)
    parser.add_argument("--queries", type=int, default=1000)
    parser.add_argument("--partitions", type=int, default=1)
    parser.add_argument("--threads", type=int, default=1)
    parser.add_argument("--retrieval", choices=["exact", "hnsw"], default="exact")
    parser.add_argument("--snapshot", action="store_true", help="also time save/load and rebuild")
    args = parser.parse_args()
    retrieval: Literal["exact", "hnsw"] = args.retrieval
    vocabulary = json.loads((args.model / "tokenizer.json").read_text())["model"]["vocab"]
    words = sorted(
        word for word in vocabulary if word.isascii() and word.isalpha() and 4 <= len(word) <= 12
    )
    rng = random.Random(16)
    words = rng.sample(words, 512)
    records: list[dict[str, Any]] = [
        {
            "id": i,
            "tenant": i % args.partitions,
            "name": "".join(rng.choices("abcdefghijklmnopqrstuvwxyz", k=20)),
            "description": " ".join(rng.sample(words, 32)),
        }
        for i in range(args.records)
    ]
    finder = Finder(
        schema=Schema(
            id_field="id",
            partition_by=["tenant"],
            fields={"name": Text(mode="lexical"), "description": Text(mode="semantic")},
        ),
        model_path=args.model,
        offline=True,
        threads=args.threads,
        max_records=args.records,
        embedding_cache_bytes=0,
        retrieval=retrieval,
    )
    timings = {}
    start = time.perf_counter_ns()
    finder.add(records)
    timings["add_including_model_load_ms"] = (time.perf_counter_ns() - start) / 1e6
    stats = finder.stats()
    indices = random.Random(161).choices(range(args.records), k=args.queries)
    queries = [{**records[i], "id": -1, "name": "!" + records[i]["name"][1:]} for i in indices]
    finder.search(queries[0], min_score=0.9)
    latencies = []
    hits = false_positives = approximate = saturated = 0
    for i, query in zip(indices, queries, strict=True):
        start = time.perf_counter_ns()
        report = finder.search(query, min_score=0.9, limit=100)
        latencies.append((time.perf_counter_ns() - start) / 1e6)
        ids = {match.right_id for match in report.matches}
        hits += int(i in ids)
        false_positives += len(ids - {i})
        approximate += int(report.search_profile == "approximate")
        saturated += int(report.candidate_truncated)
    oracle_equal = 0
    for query in queries[:10]:
        bounded = finder.search(query, min_score=0.9, limit=100)
        oracle = finder.search(query, min_score=0.9, limit=100, exhaustive=True)
        oracle_equal += int(bounded.matches == oracle.matches)
    batch = [{**record, "name": "!" + record["name"][1:]} for record in records[:100]]
    start = time.perf_counter_ns()
    finder.upsert(batch)
    timings["upsert_100_unchanged_vectors_ms"] = (time.perf_counter_ns() - start) / 1e6
    start = time.perf_counter_ns()
    finder.remove(list(range(args.records - 100, args.records)))
    timings["remove_100_ms"] = (time.perf_counter_ns() - start) / 1e6
    if args.snapshot:
        with tempfile.TemporaryDirectory(prefix="univo-ann-bench-") as directory:
            path = Path(directory) / "index"
            start = time.perf_counter_ns()
            finder.save(path)
            timings["save_ms"] = (time.perf_counter_ns() - start) / 1e6
            start = time.perf_counter_ns()
            restored = Finder.load(path, offline=True, threads=args.threads)
            timings["load_including_graph_rebuild_ms"] = (time.perf_counter_ns() - start) / 1e6
            query = queries[0]
            assert (
                restored.search(query, min_score=0.9, exhaustive=True).matches
                == finder.search(query, min_score=0.9, exhaustive=True).matches
            )
    ordered = sorted(latencies)
    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    print(
        json.dumps(
            {
                "records": args.records,
                "queries": args.queries,
                "partitions": args.partitions,
                "threads": args.threads,
                "retrieval": retrieval,
                "threshold": 0.9,
                "snapshot_measured": args.snapshot,
                "timings_ms": timings,
                "search_p50_ms": statistics.median(ordered),
                "search_p95_ms": ordered[(len(ordered) * 95 + 99) // 100 - 1],
                "search_p99_ms": ordered[(len(ordered) * 99 + 99) // 100 - 1],
                "known_positive_recall": hits / args.queries,
                "other_returned_records": false_positives,
                "oracle_equal_queries": oracle_equal,
                "oracle_queries": min(10, args.queries),
                "approximate_queries": approximate,
                "saturated_queries": saturated,
                "stats": stats,
                "peak_rss_bytes": rss if platform.system() == "Darwin" else rss * 1024,
                "platform": platform.platform(),
                "python": platform.python_version(),
            }
        )
    )


if __name__ == "__main__":
    main()
