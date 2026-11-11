"""Time and memory to find duplicate pairs among synthetic company names
split into equal partitions, as a country or a tenant field splits records.

The names are those of `scale.py`, drawn for each partition with its own
seed: one record in ten repeats an earlier name of its partition with a
typo. Univo compares only records of the same partition, at 0.8, lexical or
semantic with exact retrieval, on every core. The time covers creating the
`Finder`, loading the model and `find`, not building the records.
"""

from __future__ import annotations

import argparse
import json
import platform
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from scale import cpu_seconds, names, peak_rss_mib


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("lexical", "semantic"), required=True)
    parser.add_argument("--records", type=int, required=True)
    parser.add_argument("--partitions", type=int, required=True)
    parser.add_argument("--model-path")
    arguments = parser.parse_args(argv)

    from univo import Finder, Schema, Text

    size = arguments.records // arguments.partitions
    records = [
        {"row": part * size + row, "part": part, "text": text}
        for part in range(arguments.partitions)
        for row, text in enumerate(names(size, seed=part))
    ]
    semantic = arguments.mode == "semantic"
    start, cpu = time.perf_counter(), cpu_seconds()
    finder = Finder(
        schema=Schema(
            id_field="row", partition_by=("part",), fields={"text": Text(mode=arguments.mode)}
        ),
        model_path=arguments.model_path if semantic else None,
        offline=True,
        max_records=len(records),
        max_semantic_comparisons=2**62,
    )
    report = finder.find(records, min_score=0.8, limit=2**32 - 1)
    end = time.perf_counter()
    print(
        json.dumps(
            {
                "mode": arguments.mode,
                "records": len(records),
                "partitions": arguments.partitions,
                "seconds": round(end - start, 2),
                "cpu_seconds": round(cpu_seconds() - cpu, 1),
                "pairs": report.total_matches,
                "truncated": report.candidate_truncated or report.output_truncated,
                "search_profile": report.search_profile,
                "peak_rss_mib": round(peak_rss_mib(), 1),
                "python": platform.python_version(),
                "machine": platform.machine(),
            }
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
