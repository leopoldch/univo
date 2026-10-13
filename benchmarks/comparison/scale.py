"""U-003: time and memory to deduplicate synthetic company names.

One record in ten repeats an earlier name with a typo. Each tool runs with
its documented defaults in its own process: SemHash 0.9 with its usearch
index; Univo 0.8, lexical, or semantic with exact (dense) or HNSW
retrieval. Univo returns pairs; the same greedy rule as SemHash turns them
into removals, inside the timed section.

The same steps are timed for every tool: `setup_seconds` loads the model
(Univo through a one-record warm-up, since it loads it lazily),
`dedupe_seconds` deduplicates, and `total_seconds` is both. CPU seconds
(user and system, every thread) are measured over the deduplication, so
that a tool using several cores shows it.
"""

from __future__ import annotations

import argparse
import json
import platform
import random
import resource
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dedupe import greedy

SYLLABLES = [
    "ka",
    "lo",
    "mi",
    "ne",
    "ra",
    "to",
    "vi",
    "su",
    "dor",
    "pel",
    "ban",
    "tri",
    "gon",
    "zu",
    "fa",
    "re",
    "qui",
    "mo",
    "sel",
    "an",
    "ex",
    "or",
    "il",
    "ty",
    "cha",
    "vel",
]


def names(count: int) -> list[str]:
    rng = random.Random(7)
    out: list[str] = []
    for _ in range(count):
        if out and rng.random() < 0.1:
            name = list(rng.choice(out))
            name[rng.randrange(len(name))] = rng.choice("aeiouxz")
            out.append("".join(name))
            continue
        words = [
            "".join(rng.choice(SYLLABLES) for _ in range(rng.randint(2, 3)))
            for _ in range(rng.randint(2, 3))
        ]
        out.append(" ".join(words) + f" {rng.randint(0, 999)}")
    return out


def cpu_seconds() -> float:
    usage = resource.getrusage(resource.RUSAGE_SELF)
    return usage.ru_utime + usage.ru_stime


def peak_rss_mib() -> float:
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return peak / (1 << 20) if sys.platform == "darwin" else peak / (1 << 10)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--tool", choices=("univo-lexical", "univo-semantic", "semhash"), required=True
    )
    parser.add_argument("--records", type=int, required=True)
    parser.add_argument("--model-path")
    parser.add_argument(
        "--retrieval",
        choices=("hnsw", "exact"),
        default="hnsw",
        help="Univo semantic retrieval: explicit HNSW or exhaustive dense search",
    )
    parser.add_argument(
        "--threads", type=int, default=None, help="Univo thread budget, every core by default"
    )
    arguments = parser.parse_args(argv)
    texts = names(arguments.records)

    start = time.perf_counter()
    if arguments.tool == "semhash":
        from model2vec import StaticModel
        from semhash import SemHash

        model = StaticModel.from_pretrained(arguments.model_path)
        loaded, cpu = time.perf_counter(), cpu_seconds()
        result = SemHash.from_records(
            [{"text": text} for text in texts], columns=["text"], model=model
        ).self_deduplicate(threshold=0.9)
        removed = len(result.filtered)
    else:
        from univo import Finder, Schema, Text

        semantic = arguments.tool == "univo-semantic"
        finder = Finder(
            schema=Schema(
                fields={"text": Text(mode="semantic" if semantic else "lexical")}, id_field="row"
            ),
            model_path=arguments.model_path if semantic else None,
            offline=True,
            max_records=arguments.records,
            retrieval=arguments.retrieval if semantic else "exact",
            max_semantic_comparisons=2**62,
            threads=arguments.threads,
        )
        # Loads the model, which Univo resolves on first use.
        finder.find([{"row": 0, "text": "warm-up"}])
        loaded, cpu = time.perf_counter(), cpu_seconds()
        report = finder.find(
            [{"row": row, "text": text} for row, text in enumerate(texts)],
            min_score=0.8,
            limit=2**32 - 1,
        )
        pairs = [(int(m.left_id), int(m.right_id), m.score) for m in report.matches]
        removed = len(greedy(texts, pairs, 0.8))
    end = time.perf_counter()
    print(
        json.dumps(
            {
                "tool": arguments.tool,
                "retrieval": arguments.retrieval if arguments.tool == "univo-semantic" else None,
                "records": arguments.records,
                "threads": None if arguments.tool == "semhash" else arguments.threads,
                "setup_seconds": round(loaded - start, 3),
                "dedupe_seconds": round(end - loaded, 3),
                "total_seconds": round(end - start, 3),
                "dedupe_cpu_seconds": round(cpu_seconds() - cpu, 3),
                "removed": removed,
                "peak_rss_mib": round(peak_rss_mib(), 1),
                "python": platform.python_version(),
                "machine": platform.machine(),
            }
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
