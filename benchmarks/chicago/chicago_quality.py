"""Quality of Univo on 9 public lists of Chicago's early childhood sites.

The data of the Dedupe library's own example: 3,337 rows describing 1,162
sites, each labelled with its true site. The script

1. splits the sites in two halves by a hash of their label;
2. on the validation half, tries three schemas and ten thresholds, and keeps
   the best recall with at least 95% precision;
3. measures those settings once on the test half, for `find` pairs and for
   pairs inside `group` groups;
4. groups the whole dataset, as the example `clean-a-csv` does;
5. replays the example `check-new-records`: one list stored, the rows of the
   8 others checked one at a time, each decision compared with the labels.

Usage: python benchmarks/chicago/evaluate.py [--output results.json]
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import itertools
import json
import statistics
import time
import urllib.request
from collections import Counter, defaultdict
from pathlib import Path
from typing import TypedDict

from univo import Exact, Finder, Schema, Text

URL = (
    "https://raw.githubusercontent.com/dedupeio/dedupe-examples/"
    "48916bf09205756a2948a30ec0275ac2e394288d/csv_example/csv_example_input_with_true_ids.csv"
)
SHA256 = "13cbb7b939c4b201f92b09b6e800ae034f1f783d825dda7095fdaccb2793edaa"
FIRST_LIST = "CPS_Early_Childhood_Portal_scrape"
MIN_PRECISION = 0.95
THRESHOLDS = [round(0.5 + 0.05 * step, 2) for step in range(10)]
SCHEMAS: dict[str, dict[str, Text | Exact]] = {
    "name + address": {"name": Text(), "address": Text()},
    "name + address + phone": {"name": Text(), "address": Text(), "phone": Exact()},
    "name + address + phone + zip": {
        "name": Text(),
        "address": Text(),
        "phone": Exact(),
        "zip": Exact(),
    },
}

Row = dict[str, str | None]
Pair = frozenset[str]


class Replay(TypedDict):
    checked: int
    decisions: dict[str, int]
    right: dict[str, float]
    mean_ms: float
    median_ms: float


def load() -> list[Row]:
    """The rows as the examples read them, with the true `site` of each one."""
    with urllib.request.urlopen(URL, timeout=60) as response:
        original = response.read()
    if hashlib.sha256(original).hexdigest() != SHA256:
        raise RuntimeError("unexpected file")
    text = io.StringIO(original.decode("utf-8", errors="replace"), newline="")
    rows: list[Row] = []
    for row in csv.DictReader(text):
        cleaned = {
            "id": row["Id"],
            "name": " ".join(row["Site name"].split()),
            "address": " ".join(row["Address"].split()),
            "zip": row["Zip"].strip(),
            "phone": "".join(c for c in row["Phone"] if c.isdigit())[-7:],
            "source": row["Source"].removesuffix(".csv"),
            "site": row["True Id"],
        }
        rows.append({key: value or None for key, value in cleaned.items()})
    return rows


def half(row: Row) -> str:
    digest = hashlib.sha256(str(row["site"]).encode()).digest()
    return "validation" if digest[0] % 2 == 0 else "test"


def true_pairs(rows: list[Row]) -> set[Pair]:
    sites: dict[str, list[str]] = defaultdict(list)
    for row in rows:
        sites[str(row["site"])].append(str(row["id"]))
    return {frozenset(pair) for ids in sites.values() for pair in itertools.combinations(ids, 2)}


def compared(row: Row, fields: dict[str, Text | Exact]) -> Row:
    return {key: row[key] for key in ["id", *fields]}


def found_pairs(fields: dict[str, Text | Exact], rows: list[Row], min_score: float) -> set[Pair]:
    finder = Finder(schema=Schema(id_field="id", fields=fields))
    records = [compared(row, fields) for row in rows]
    report = finder.find(records, min_score=min_score, limit=10_000_000)
    return {frozenset((str(m.left_id), str(m.right_id))) for m in report.matches}


def grouped_pairs(fields: dict[str, Text | Exact], rows: list[Row], min_score: float) -> set[Pair]:
    finder = Finder(schema=Schema(id_field="id", fields=fields))
    groups = finder.group([compared(row, fields) for row in rows], min_score=min_score)
    return {
        frozenset((str(left), str(right)))
        for group in groups.groups
        for left, right in itertools.combinations(group.members, 2)
    }


def quality(found: set[Pair], truth: set[Pair]) -> dict[str, float]:
    correct = len(found & truth)
    return {
        "pairs": len(found),
        "precision": correct / len(found) if found else 1.0,
        "recall": correct / len(truth),
    }


def replay(rows: list[Row], fields: dict[str, Text | Exact], min_score: float) -> Replay:
    """The decisions of check-new-records, compared with the labels."""
    finder = Finder(schema=Schema(id_field="id", fields=fields))
    stored = [row for row in rows if row["source"] == FIRST_LIST]
    finder.add([compared(row, fields) for row in stored])
    known = {row["site"] for row in stored}
    total: Counter[str] = Counter()
    right: Counter[str] = Counter()
    seconds: list[float] = []
    for row in rows:
        if row["source"] == FIRST_LIST:
            continue
        started = time.perf_counter()
        status = finder.check_or_add(compared(row, fields), min_score=min_score).status
        seconds.append(time.perf_counter() - started)
        was_known = row["site"] in known
        total[status] += 1
        right[status] += (not was_known) if status == "inserted" else was_known
        if status == "inserted":
            known.add(row["site"])
    return {
        "checked": len(seconds),
        "decisions": dict(total),
        "right": {status: right[status] / total[status] for status in total},
        "mean_ms": statistics.mean(seconds) * 1e3,
        "median_ms": statistics.median(seconds) * 1e3,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=Path, default=Path(__file__).parent / "results.json")
    arguments = parser.parse_args()

    rows = load()
    halves = {name: [row for row in rows if half(row) == name] for name in ("validation", "test")}
    truth = {name: true_pairs(part) for name, part in halves.items()}

    curves = {
        label: [
            {
                "min_score": t,
                **quality(found_pairs(fields, halves["validation"], t), truth["validation"]),
            }
            for t in THRESHOLDS
        ]
        for label, fields in SCHEMAS.items()
    }
    _, schema, min_score = max(
        (point["recall"], label, point["min_score"])
        for label, points in curves.items()
        for point in points
        if point["precision"] >= MIN_PRECISION
    )
    fields = SCHEMAS[schema]

    started = time.perf_counter()
    groups = Finder(schema=Schema(id_field="id", fields=fields)).group(
        [compared(row, fields) for row in rows], min_score=min_score
    )
    group_seconds = time.perf_counter() - started
    in_groups = sum(len(group.members) for group in groups.groups)

    test_find = quality(found_pairs(fields, halves["test"], min_score), truth["test"])
    test_group = quality(grouped_pairs(fields, halves["test"], min_score), truth["test"])
    replayed = replay(rows, fields, min_score)
    results = {
        "records": len(rows),
        "sites": len({row["site"] for row in rows}),
        "rule": f"best validation recall with precision >= {MIN_PRECISION:.0%}",
        "chosen": {"schema": schema, "min_score": min_score},
        "validation_curves": curves,
        "test_find": test_find,
        "test_group": test_group,
        "whole_dataset": {
            "groups": len(groups.groups),
            "rows_once_merged": len(rows) - in_groups + len(groups.groups),
            "seconds": group_seconds,
        },
        "check_new_records": replayed,
    }
    arguments.output.write_text(json.dumps(results, indent=2) + "\n")

    print(f"Chosen on validation: {schema}, min_score {min_score}")
    for label, measured in (("find pairs", test_find), ("pairs in groups", test_group)):
        precision, recall = measured["precision"], measured["recall"]
        print(f"Test half, {label}: precision {precision:.1%}, recall {recall:.1%}")
    print(f"Whole dataset: {len(groups.groups)} groups in {group_seconds:.2f} s")
    print(f"check_or_add decisions: {replayed['decisions']}")
    print(f"  right: {replayed['right']}")
    print(f"  median {replayed['median_ms']:.2f} ms, mean {replayed['mean_ms']:.2f} ms")


if __name__ == "__main__":
    main()
