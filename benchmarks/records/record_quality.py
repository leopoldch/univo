"""Precision and recall of the pairs each tool found on the record datasets.

Reads the pair files written by the `record_pairs_*.py` scripts. For every tool and
dataset:

1. on the **validation** half, each variant of the tool gets the threshold
   with the best F1, and the tool keeps its variant with the best F1;
2. on the **test** half, that variant and that threshold are measured once.

The same is done with two other rules:

- **precise**: the best recall with a precision of at least 95%;
- **entities**, for deduplication: the pairs kept at a threshold are closed
  transitively into groups, as a deduplicated table needs, and every pair of
  records of one group counts as found. One wrong pair then joins two
  entities. Thresholds are tried at 60 quantiles of the scores.

Scores are only compared with one another: any increasing score works. A pair
counts when both records are in the half; the true pairs are those of
`record_datasets.py`.

    python benchmarks/records/record_quality.py --cache /tmp/records-data \
        --pairs /tmp/records-pairs --output benchmarks/records/results.json
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from record_datasets import Pair, load, pair

MIN_PRECISION = 0.95
QUANTILES = 60
Scored = dict[Pair, float]


def measure(found: Scored, truth: set[Pair], threshold: float) -> dict[str, float]:
    kept = [key for key, score in found.items() if score >= threshold]
    correct = sum(key in truth for key in kept)
    precision = correct / len(kept) if kept else 1.0
    recall = correct / len(truth)
    f1 = 2 * precision * recall / (precision + recall) if correct else 0.0
    return {
        "threshold": threshold,
        "pairs": len(kept),
        "precision": precision,
        "recall": recall,
        "f1": f1,
    }


def thresholds(found: Scored, truth: set[Pair]) -> dict[str, dict[str, float] | None]:
    """The thresholds with the best F1, and the best recall at 95% precision."""
    ranked = sorted(found.items(), key=lambda item: -item[1])
    best: dict[str, dict[str, float] | None] = {"f1": None, "precise": None}
    correct = 0
    for position, (key, score) in enumerate(ranked, start=1):
        correct += key in truth
        if position < len(ranked) and ranked[position][1] == score:
            continue
        precision, recall = correct / position, correct / len(truth)
        point = {
            "threshold": score,
            "pairs": position,
            "precision": precision,
            "recall": recall,
            "f1": 2 * precision * recall / (precision + recall) if correct else 0.0,
        }
        if best["f1"] is None or point["f1"] > best["f1"]["f1"]:
            best["f1"] = point
        precise = best["precise"]
        if precision >= MIN_PRECISION and (precise is None or recall > precise["recall"]):
            best["precise"] = point
    return best


def grouped(
    found: Scored, entity: dict[str, str], true_pairs: int, threshold: float
) -> dict[str, float]:
    """Precision and recall of the groups the pairs kept at `threshold` form."""
    parent: dict[str, str] = {}

    def root(node: str) -> str:
        while parent.setdefault(node, node) != node:
            parent[node] = node = parent[parent[node]]
        return node

    for (left, right), score in found.items():
        if score >= threshold:
            parent[root(left)] = root(right)
    groups: dict[str, list[str]] = defaultdict(list)
    for node in list(parent):
        groups[root(node)].append(node)
    pairs = sum(len(group) * (len(group) - 1) // 2 for group in groups.values())
    correct = sum(
        count * (count - 1) // 2
        for group in groups.values()
        for count in Counter(entity[node] for node in group).values()
    )
    precision = correct / pairs if pairs else 1.0
    recall = correct / true_pairs
    return {
        "threshold": threshold,
        "pairs": pairs,
        "precision": precision,
        "recall": recall,
        "f1": 2 * precision * recall / (precision + recall) if correct else 0.0,
        "largest_group": max((len(group) for group in groups.values()), default=0),
    }


def best_groups(found: Scored, entity: dict[str, str], true_pairs: int) -> dict[str, float] | None:
    scores = sorted(set(found.values()))
    if not scores:
        return None
    tried = {scores[step * (len(scores) - 1) // QUANTILES] for step in range(QUANTILES + 1)}
    return max(
        (grouped(found, entity, true_pairs, threshold) for threshold in sorted(tried)),
        key=lambda point: point["f1"],
    )


def evaluate(path: Path, cache: Path) -> dict[str, Any]:
    result = json.loads(path.read_text())
    dataset = load(result["dataset"], cache)
    truth = {half: dataset.truth_of(half) for half in ("validation", "test")}
    entity = {str(record["id"]): str(record["entity"]) for record in dataset.records}
    runs: dict[str, dict[str, dict[str, Any]]] = {}
    for run in result["runs"]:
        ids = {record["id"] for record in dataset.half(run["half"])}
        found = {
            pair(left, right): score
            for left, right, score in run["pairs"]
            if left in ids and right in ids
        }
        runs.setdefault(run["variant"], {})[run["half"]] = {
            "found": found,
            "seconds": run["seconds"],
        }
    # Every variant: its thresholds from validation, measured on test.
    by_variant: dict[str, dict[str, Any]] = {}
    for variant, halves in runs.items():
        validation, test = halves["validation"]["found"], halves["test"]
        measured: dict[str, Any] = {"test_seconds": test["seconds"]}
        for rule, point in thresholds(validation, truth["validation"]).items():
            measured[rule] = point and {
                "validation": point,
                "test": measure(test["found"], truth["test"], point["threshold"]),
            }
        groups = None
        if not dataset.linking:
            groups = best_groups(validation, entity, len(truth["validation"]))
        measured["entities"] = groups and {
            "validation": groups,
            "test": grouped(test["found"], entity, len(truth["test"]), groups["threshold"]),
        }
        measured["test_pairs_written"] = len(test["found"])
        measured["test_recall_of_pairs_written"] = sum(
            key in truth["test"] for key in test["found"]
        ) / len(truth["test"])
        by_variant[variant] = measured

    # The tool's variant for each rule is the best one on validation.
    summary: dict[str, Any] = {
        "tool": result["tool"],
        "version": result["version"],
        "dataset": dataset.name,
        "records": {half: len(dataset.half(half)) for half in truth},
        "true_pairs": {half: len(pairs) for half, pairs in truth.items()},
    }
    for rule, key in (("f1", "f1"), ("precise", "recall"), ("entities", "f1")):
        ranked = [
            (measured[rule]["validation"][key], variant)
            for variant, measured in by_variant.items()
            if measured[rule] is not None
        ]
        summary[rule] = None
        if ranked:
            _, variant = max(ranked)
            chosen = by_variant[variant]
            summary[rule] = {
                "variant": variant,
                **chosen[rule],
                "test_seconds": chosen["test_seconds"],
                "test_pairs_written": chosen["test_pairs_written"],
                "test_recall_of_pairs_written": chosen["test_recall_of_pairs_written"],
            }
    summary["by_variant"] = by_variant
    return summary


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--pairs", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    summaries = [evaluate(path, arguments.cache) for path in sorted(arguments.pairs.glob("*.json"))]
    arguments.output.write_text(json.dumps(summaries, indent=2) + "\n")
    print("| Dataset | Tool | Pairs: precision, recall | Pairs: F1 | Entities: F1 | Seconds |")
    print("| --- | --- | --- | --- | --- | --- |")
    for summary in summaries:
        best, entities = summary["f1"], summary["entities"]
        test = best["test"]
        groups = (
            "linking"
            if entities is None
            else f"{entities['test']['f1']:.3f} ({entities['test']['precision']:.1%}, "
            f"{entities['test']['recall']:.1%})"
        )
        print(
            f"| {summary['dataset']} | {summary['tool']} {summary['version']}: {best['variant']} "
            f"| {test['precision']:.1%}, {test['recall']:.1%} | {test['f1']:.3f} "
            f"| {groups} | {best['test_seconds']:.1f} |"
        )


if __name__ == "__main__":
    main()
