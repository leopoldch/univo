"""Univo on the record datasets, as a user would run it today.

Each dataset has a few schemas, written from the meaning of its columns.
Deduplication calls `find` on the records; linking adds the first source,
then calls `search` for each record of the second, keeping its ten best
matches. Each schema gives two rankings: the score alone, and the score
multiplied by the coverage, the share of the schema that gave evidence.
Which schema and ranking is used is chosen on the validation half by
`record_quality.py`.

Writes every pair scoring at least `--floor`, and the time from the records
in memory to the pairs in memory. With `--group`, writes instead the pairs
inside the groups of `group` at five thresholds: Univo's own grouping, which
joins records only when every pair of the group reaches the threshold.

With `--probability`, pairs are decided by weights learned from the records
(`find(min_probability=...)`) and ranked by their weight in bits, down to a
probability of 0.05. Linking then calls `find` on both sources together and
keeps the pairs across them.

    python -I benchmarks/records/record_pairs_univo.py --dataset febrl3 --cache /tmp/records-data \
        --output /tmp/records-pairs/febrl3-univo.json
"""

from __future__ import annotations

import argparse
import itertools
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import univo
from record_datasets import HALVES, Dataset, Record, load
from univo import Exact, Finder, Schema, Text

Fields = dict[str, Text | Exact]


def text(*names: str) -> Fields:
    return {name: Text() for name in names}


def semantic(*names: str) -> Fields:
    return {name: Text(mode="semantic") for name in names}


def exact(*names: str) -> Fields:
    return {name: Exact() for name in names}


PERSON = ("given_name", "surname", "date_of_birth", "soc_sec_id")
ADDRESS = ("street_number", "address_1", "address_2", "suburb", "postcode")
FEBRL: dict[str, Fields] = {
    "every column as text": text(*PERSON, *ADDRESS, "state"),
    "text, state exact": text(*PERSON, *ADDRESS) | exact("state"),
    "person and postcode": text(*PERSON, "postcode"),
}
SCHEMAS: dict[str, dict[str, Fields]] = {
    "febrl3": FEBRL,
    "febrl4": FEBRL,
    "historical_50k": {
        "names, birth, postcode": text(
            "first_name", "surname", "dob", "postcode_fake", "birth_place"
        ),
        "names, birth, postcode, occupation": text(
            "first_name", "surname", "dob", "postcode_fake", "birth_place"
        )
        | exact("occupation"),
        "full name, birth, postcode": text("full_name", "dob", "postcode_fake", "birth_place"),
    },
    "dblp_acm": {
        "title, authors": text("title", "authors"),
        "title, authors, year": text("title", "authors") | exact("year"),
        "title, authors, venue, year": text("title", "authors", "venue") | exact("year"),
        "title by meaning, authors, year": semantic("title") | text("authors") | exact("year"),
    },
    "abt_buy": {
        "name": text("name"),
        "name by meaning": semantic("name"),
        "name and description by meaning": semantic("name", "description"),
        "name by meaning, price": semantic("name") | exact("price"),
    },
}
LINK_LIMIT = 10
GROUP_THRESHOLDS = (0.6, 0.65, 0.7, 0.75, 0.8)
MIN_PROBABILITY = 0.05


def scored(
    dataset: Dataset, fields: Fields, records: list[Record], floor: float, model: str | None
) -> list[tuple[str, str, float, float]]:
    """Pairs `(left, right, score, coverage)` of one set of records."""
    finder = Finder(
        schema=Schema(id_field="id", fields=fields),
        model_path=model,
        offline=True,
        max_records=len(records),
        max_semantic_comparisons=2**62,
    )
    rows = [{key: record[key] for key in ("id", *fields)} for record in records]
    if not dataset.linking:
        report = finder.find(rows, min_score=floor, limit=2**31 - 1)
        return [(str(m.left_id), str(m.right_id), m.score, m.coverage) for m in report.matches]
    finder.add([row for row, record in zip(rows, records, strict=True) if record["source"] == "a"])
    pairs = []
    for row, record in zip(rows, records, strict=True):
        if record["source"] == "b":
            report = finder.search(row, min_score=floor, limit=LINK_LIMIT)
            pairs += [
                (str(m.right_id), str(row["id"]), m.score, m.coverage) for m in report.matches
            ]
    return pairs


def in_groups(fields: Fields, records: list[Record], min_score: float) -> list[list[object]]:
    """Pairs of records that `group` puts in one group, each with a score of 1."""
    finder = Finder(schema=Schema(id_field="id", fields=fields), max_records=len(records))
    rows = [{key: record[key] for key in ("id", *fields)} for record in records]
    groups = finder.group(rows, min_score=min_score, max_group_size=1000, max_pairs=10**8)
    return [
        [str(left), str(right), 1.0]
        for group in groups.groups
        for left, right in itertools.combinations(group.members, 2)
    ]


def weighed(
    dataset: Dataset, fields: Fields, records: list[Record], model: str | None
) -> list[list[object]]:
    """Pairs `[left, right, bits]` decided by learned weights."""
    finder = Finder(
        schema=Schema(id_field="id", fields=fields),
        model_path=model,
        offline=True,
        max_records=len(records),
        max_semantic_comparisons=2**62,
    )
    rows = [{key: record[key] for key in ("id", *fields)} for record in records]
    source = {str(record["id"]): record["source"] for record in records}
    report = finder.find(rows, min_probability=MIN_PROBABILITY, limit=2**31 - 1)
    return [
        [str(m.left_id), str(m.right_id), m.weight]
        for m in report.matches
        if not dataset.linking or source[str(m.left_id)] != source[str(m.right_id)]
    ]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--group", action="store_true", help="measure `group`, not `find`")
    parser.add_argument(
        "--probability", action="store_true", help="decide by learned weights, not by score"
    )
    parser.add_argument("--dataset", choices=sorted(SCHEMAS), required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--floor", type=float, default=0.5, help="lowest score kept")
    parser.add_argument("--model-path", help="the default model's directory, for semantic fields")
    arguments = parser.parse_args()

    dataset = load(arguments.dataset, arguments.cache)
    runs = []
    grouping = [
        (variant, fields, threshold)
        for variant, fields in SCHEMAS[dataset.name].items()
        for threshold in GROUP_THRESHOLDS
        if arguments.group and not dataset.linking and "semantic" not in variant
    ]
    for variant, fields, threshold in grouping:
        for half in HALVES:
            started = time.perf_counter()
            inside = in_groups(fields, dataset.half(half), threshold)
            seconds = time.perf_counter() - started
            name = f"{variant}, at {threshold}"
            runs.append({"variant": name, "half": half, "seconds": seconds, "pairs": inside})
            print(f"{dataset.name} {half}, {name}: {len(inside)} pairs in {seconds:.1f} s")
    for variant, fields in SCHEMAS[dataset.name].items() if arguments.probability else ():
        for half in HALVES:
            started = time.perf_counter()
            inside = weighed(dataset, fields, dataset.half(half), arguments.model_path)
            seconds = time.perf_counter() - started
            runs.append({"variant": variant, "half": half, "seconds": seconds, "pairs": inside})
            print(f"{dataset.name} {half}, {variant}: {len(inside)} pairs in {seconds:.1f} s")
    plain = not (arguments.group or arguments.probability)
    for variant, fields in SCHEMAS[dataset.name].items() if plain else ():
        for half in HALVES:
            started = time.perf_counter()
            pairs = scored(
                dataset, fields, dataset.half(half), arguments.floor, arguments.model_path
            )
            seconds = time.perf_counter() - started
            rankings = {
                variant: [[a, b, round(score, 6)] for a, b, score, _ in pairs],
                f"{variant}, by coverage": [
                    [a, b, round(score * coverage, 6)] for a, b, score, coverage in pairs
                ],
            }
            for name, ranked in rankings.items():
                runs.append({"variant": name, "half": half, "seconds": seconds, "pairs": ranked})
            print(f"{dataset.name} {half}, {variant}: {len(pairs)} pairs in {seconds:.1f} s")
    result = {
        "tool": "univo group"
        if arguments.group
        else ("univo by probability" if arguments.probability else "univo"),
        "version": univo.__version__,
        "dataset": dataset.name,
        "floor": arguments.floor,
        "runs": runs,
    }
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(json.dumps(result) + "\n")


if __name__ == "__main__":
    main()
