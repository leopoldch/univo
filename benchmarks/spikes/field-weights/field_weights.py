"""Spike: can weights learned without labels turn Univo's field similarities
into better decisions?

Univo scores a pair by the weighted mean of its field similarities. This
prototype keeps Univo's candidates and field similarities and replaces the
mean by the weights of probabilistic record linkage (Fellegi and Sunter,
1969), learned from the records themselves:

1. **Levels.** A field similarity becomes one of four levels, fixed here for
   every field and dataset: equal (1), close (at least 0.88), partial (at
   least 0.7), different. A missing value gives no evidence, as in Univo.
2. **u**, how often a level occurs between two different entities: counted
   over every pair of 1,500 records drawn at random. Equal values use the
   frequency of the value instead, so that sharing a rare value weighs more
   than sharing a common one.
3. **m**, how often a level occurs between two records of one entity: the
   candidates are split in two classes by expectation maximisation, and the
   class that agrees gives m. The other class is not used: candidates were
   selected because they agree, so it does not describe unrelated records.
4. **Weight** of a pair: the sum over its fields of log2(m / u).

No label is read. The pairs are written for `benchmarks/records/record_quality.py`,
next to the same candidates ranked by Univo's own score.

    python -I benchmarks/spikes/field-weights/field_weights.py --dataset historical_50k \
        --cache /tmp/records-data --output /tmp/records-pairs/historical_50k-weights.json
"""

from __future__ import annotations

import argparse
import json
import math
import random
import sys
import time
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "records"))
from record_datasets import HALVES, Dataset, Record, load
from univo import Exact, Finder, Schema, Text

LEVELS = ("equal", "close", "partial", "different")
SAMPLE = 1500
ROUNDS = 40
LINK_LIMIT = 10
Fields = dict[str, Text | Exact]


def text(*names: str) -> Fields:
    return {name: Text() for name in names}


FEBRL = text(
    "given_name",
    "surname",
    "date_of_birth",
    "soc_sec_id",
    "street_number",
    "address_1",
    "address_2",
    "suburb",
    "postcode",
    "state",
)
SCHEMAS: dict[str, Fields] = {
    "febrl3": FEBRL,
    "febrl4": FEBRL,
    "historical_50k": text("first_name", "surname", "dob", "postcode_fake", "birth_place")
    | {"occupation": Exact()},
    "dblp_acm": text("title", "authors", "venue") | {"year": Exact()},
    "abt_buy": {
        "name": Text(mode="semantic"),
        "description": Text(mode="semantic"),
        "price": Exact(),
    },
}


def level(similarity: float | None) -> str | None:
    if similarity is None:
        return None
    if similarity >= 0.9999:
        return "equal"
    if similarity >= 0.88:
        return "close"
    return "partial" if similarity >= 0.7 else "different"


def weighted(
    dataset: Dataset, records: list[Record], floor: float, model: str | None
) -> tuple[list[list[object]], list[list[object]], dict[str, dict[str, float]]]:
    """The candidates of `records` by weight and by Univo's score, and the weights."""
    fields = SCHEMAS[dataset.name]
    names = list(fields)
    rows = [{key: record[key] for key in ("id", *names)} for record in records]
    value = {str(row["id"]): row for row in rows}
    finder = Finder(
        schema=Schema(id_field="id", fields=fields),
        model_path=model,
        offline=True,
        max_records=len(rows),
        max_semantic_comparisons=2**62,
        max_exhaustive_pairs=SAMPLE * SAMPLE,
    )

    # Candidates and their field similarities, as Univo finds them today.
    candidates: list[tuple[str, str, float, list[str | None]]] = []
    if dataset.linking:
        stored = [row for row, record in zip(rows, records, strict=True) if record["source"] == "a"]
        finder.add(stored)
        for row, record in zip(rows, records, strict=True):
            if record["source"] == "b":
                for match in finder.search(row, min_score=floor, limit=LINK_LIMIT).matches:
                    levels = [level(match.field_scores.get(name)) for name in names]
                    candidates.append((str(match.right_id), str(row["id"]), match.score, levels))
        finder.remove([str(row["id"]) for row in stored])
    else:
        for match in finder.find(rows, min_score=floor, limit=2**31 - 1).matches:
            levels = [level(match.field_scores.get(name)) for name in names]
            candidates.append((str(match.left_id), str(match.right_id), match.score, levels))

    # u: levels between records drawn at random, nearly all different entities.
    sample = random.Random(1).sample(rows, min(SAMPLE, len(rows)))
    seen = {name: Counter[str]() for name in names}
    for match in finder.find(sample, min_score=1e-9, limit=2**31 - 1, exhaustive=True).matches:
        for name in names:
            found = level(match.field_scores.get(name))
            if found in ("equal", "close", "partial"):
                seen[name][found] += 1
    u: dict[str, dict[str, float]] = {}
    for name in names:
        present = sum(row[name] is not None for row in sample)
        pairs = max(present * (present - 1) / 2, 1.0)
        u[name] = {found: max(seen[name][found], 0.5) / pairs for found in LEVELS[:3]}
        u[name]["different"] = max(1.0 - sum(u[name].values()), 1e-6)
    frequency = {
        name: Counter(str(row[name]).casefold() for row in rows if row[name] is not None)
        for name in names
    }
    total = {name: max(sum(frequency[name].values()), 2) for name in names}

    def chance(name: str, found: str, left: str) -> float:
        if found != "equal":
            return u[name][found]
        count = frequency[name][str(value[left][name]).casefold()]
        return max((count - 1) / (total[name] - 1), 1.0 / total[name])

    # m: two classes among the candidates, by expectation maximisation.
    m = {name: dict(zip(LEVELS, (0.6, 0.2, 0.12, 0.08), strict=True)) for name in names}
    other = {name: dict(zip(LEVELS, (0.15, 0.05, 0.2, 0.6), strict=True)) for name in names}
    share = 0.3
    for _ in range(ROUNDS):
        agree = {name: dict.fromkeys(LEVELS, 0.0) for name in names}
        differ = {name: dict.fromkeys(LEVELS, 0.0) for name in names}
        matched = 0.0
        for _, _, _, levels in candidates:
            same, distinct = share, 1.0 - share
            for name, found in zip(names, levels, strict=True):
                if found:
                    same *= m[name][found]
                    distinct *= other[name][found]
            probability = same / (same + distinct) if same + distinct > 0 else 0.0
            matched += probability
            for name, found in zip(names, levels, strict=True):
                if found:
                    agree[name][found] += probability
                    differ[name][found] += 1.0 - probability
        share = min(max(matched / max(len(candidates), 1), 1e-6), 1.0 - 1e-6)
        for name in names:
            for counts, table in ((agree[name], m[name]), (differ[name], other[name])):
                evidence = sum(counts.values())
                if evidence > 0:
                    for found in LEVELS:
                        table[found] = max(counts[found] / evidence, 1e-4)

    by_weight: list[list[object]] = []
    by_score: list[list[object]] = []
    for left, right, score, levels in candidates:
        weight = sum(
            math.log2(m[name][found] / chance(name, found, left))
            for name, found in zip(names, levels, strict=True)
            if found
        )
        by_weight.append([left, right, round(weight, 4)])
        by_score.append([left, right, round(score, 6)])
    weights = {
        name: {found: round(math.log2(m[name][found] / u[name][found]), 2) for found in LEVELS}
        for name in names
    }
    return by_weight, by_score, weights


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dataset", choices=sorted(SCHEMAS), required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--floor", type=float, default=0.5, help="lowest Univo score kept")
    parser.add_argument("--model-path", help="the default model's directory, for semantic fields")
    arguments = parser.parse_args()

    dataset = load(arguments.dataset, arguments.cache)
    runs, learned = [], {}
    for half in HALVES:
        started = time.perf_counter()
        by_weight, by_score, weights = weighted(
            dataset, dataset.half(half), arguments.floor, arguments.model_path
        )
        seconds = time.perf_counter() - started
        learned[half] = weights
        for variant, pairs in (("learned weights", by_weight), ("univo score", by_score)):
            runs.append({"variant": variant, "half": half, "seconds": seconds, "pairs": pairs})
        print(f"{dataset.name} {half}: {len(by_weight)} candidates in {seconds:.1f} s")
    for name, table in learned["test"].items():
        print(f"  {name}: {table}")
    result = {
        "tool": "univo candidates, learned weights",
        "version": "spike",
        "dataset": dataset.name,
        "floor": arguments.floor,
        "weights": learned,
        "runs": runs,
    }
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(json.dumps(result) + "\n")


if __name__ == "__main__":
    main()
