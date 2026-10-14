"""U-003: the same deduplication task for Univo and SemHash.

Each tool deduplicates the organization names of the ROR name-alias
benchmark (U-012), one split at a time: records sharing a ROR ID are
duplicates. Every tool is held to SemHash's own decision rule (`greedy`):
identical texts form one group, and in order of first occurrence a group
is removed when an earlier selected group reaches the threshold, pointing
to the most similar one. The threshold is chosen
on validation with the U-012 rule (highest recall at precision >= 98% with
at least 20 correct removals, higher threshold on ties) and then applied
once to test.

Run each tool in its own process. Univo needs an installed wheel; SemHash
needs `semhash` in a separate environment. Both read the same local model.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import resource
import sys
import time
from collections import Counter, defaultdict
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import Any, Literal

THRESHOLDS = [round(0.5 + step / 100, 2) for step in range(51)]
MIN_PRECISION = 0.98
MIN_CORRECT = 20
TOOLS = ("univo-exact", "univo-lexical", "univo-semantic", "semhash")

# One record: its position, ROR ID and name.
Record = tuple[int, str, str]
# A removal: the record removed and the kept record it duplicates.
Removal = tuple[int, int]


def load_splits(path: Path) -> dict[str, list[Record]]:
    """The distinct (ROR ID, name) records of each split, in a fixed order
    that no tool chooses: the SHA-256 of the record."""
    seen: dict[str, set[tuple[str, str]]] = defaultdict(set)
    with path.open(encoding="utf-8") as lines:
        for line in lines:
            pair = json.loads(line)
            seen[pair["split"]].add((pair["left_entity"], pair["left"]))
            seen[pair["split"]].add((pair["right_entity"], pair["right"]))
    splits = {}
    for split in ("validation", "test"):
        ordered = sorted(
            seen[split], key=lambda item: hashlib.sha256(f"{item[0]}\t{item[1]}".encode()).digest()
        )
        splits[split] = [(row, entity, text) for row, (entity, text) in enumerate(ordered)]
    return splits


def greedy(
    texts: Sequence[str], pairs: Iterable[tuple[int, int, float]], threshold: float
) -> list[Removal]:
    """SemHash 0.5.0's selection. Records with identical text form one group,
    ordered by first occurrence. In that order, a group is removed when an
    earlier selected group reaches `threshold`, pointing to the most similar
    one (earliest on ties); otherwise it is selected. A removed group's
    records all point to the chosen group's first record; a selected group's
    other copies point to its own first record."""
    group_of: dict[str, int] = {}
    members: list[list[int]] = []
    for row, text in enumerate(texts):
        if text not in group_of:
            group_of[text] = len(members)
            members.append([])
        members[group_of[text]].append(row)
    best: dict[tuple[int, int], float] = {}
    for left, right, score in pairs:
        a, b = group_of[texts[left]], group_of[texts[right]]
        if a != b and score >= threshold:
            key = (min(a, b), max(a, b))
            best[key] = max(score, best.get(key, score))
    earlier: dict[int, list[tuple[int, float]]] = defaultdict(list)
    for (first, second), score in best.items():
        earlier[second].append((first, score))
    selected = [False] * len(members)
    removals: list[Removal] = []
    for group, rows in enumerate(members):
        matches = [(score, -other) for other, score in earlier[group] if selected[other]]
        if matches:
            _, canonical = max(matches)
            removals.extend((row, members[-canonical][0]) for row in rows)
        else:
            selected[group] = True
            removals.extend((row, rows[0]) for row in rows[1:])
    return removals


def metrics(records: Sequence[Record], removals: Sequence[Removal]) -> dict[str, float | int]:
    """Removals are correct when both records share a ROR ID; every record
    beyond the first of its entity could be removed."""
    entity = {row: ror for row, ror, _ in records}
    possible = sum(count - 1 for count in Counter(entity.values()).values())
    correct = sum(entity[row] == entity[kept] for row, kept in removals)
    removed = len(removals)
    return {
        "removed": removed,
        "correct": correct,
        "possible": possible,
        "precision": correct / removed if removed else 1.0,
        "recall": correct / possible if possible else 0.0,
    }


def calibrate(curve: dict[float, dict[str, float | int]]) -> float | None:
    """The U-012 rule: highest recall at precision >= 98% with at least 20
    correct removals; higher thresholds win ties; None when none qualifies."""
    eligible = [
        (point["recall"], threshold)
        for threshold, point in curve.items()
        if point["precision"] >= MIN_PRECISION and point["correct"] >= MIN_CORRECT
    ]
    return max(eligible)[1] if eligible else None


def univo_removals(
    records: Sequence[Record], mode: str, model_path: str | None
) -> dict[float, list[Removal]]:
    from univo import Finder, Schema, Text

    text_mode: Literal["lexical", "semantic"] = (
        "semantic" if mode == "univo-semantic" else "lexical"
    )
    finder = Finder(
        schema=Schema(fields={"text": Text(mode=text_mode)}, id_field="row"),
        model_path=model_path if text_mode == "semantic" else None,
        offline=True,
        max_records=max(len(records), 1),
        max_semantic_comparisons=2**62,
    )
    report = finder.find(
        [{"row": row, "text": text} for row, _, text in records],
        min_score=min(THRESHOLDS),
        limit=2**32 - 1,
    )
    if report.candidate_truncated or report.output_truncated:
        raise RuntimeError("the search was truncated; raise its limits")
    pairs = [
        (int(match.left_id), int(match.right_id), match.score)
        for match in report.matches
        if mode != "univo-exact" or match.kind == "exact"
    ]
    if mode == "univo-exact":
        pairs = [(left, right, 1.0) for left, right, _ in pairs]
    texts = [text for _, _, text in records]
    return {threshold: greedy(texts, pairs, threshold) for threshold in THRESHOLDS}


def semhash_removals(records: Sequence[Record], model_path: str) -> dict[float, list[Removal]]:
    from model2vec import StaticModel
    from semhash import SemHash

    model = StaticModel.from_pretrained(model_path)
    index = SemHash.from_records(
        [{"text": text, "row": row} for row, _, text in records], columns=["text"], model=model
    )
    result = index.self_deduplicate(threshold=min(THRESHOLDS))
    removals = {}
    for threshold in THRESHOLDS:
        result.rethreshold(threshold)
        removals[threshold] = [
            (duplicate.record["row"], duplicate.duplicate_of["row"])
            for duplicate in result.filtered
        ]
    return removals


def peak_rss_mib() -> float:
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    # Bytes on macOS, kibibytes on Linux.
    return peak / (1 << 20) if sys.platform == "darwin" else peak / (1 << 10)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "pairs", type=Path, help="ror-pairs.jsonl from tools/quality/build_quality_corpus.py"
    )
    parser.add_argument("--tool", choices=TOOLS, required=True)
    parser.add_argument("--model-path", help="local potion-base-8M directory, for semantic tools")
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args(argv)
    if arguments.tool in ("univo-semantic", "semhash") and not arguments.model_path:
        parser.error("--model-path is required for semantic tools")

    splits = load_splits(arguments.pairs)
    curves: dict[str, dict[float, dict[str, float | int]]] = {}
    seconds: dict[str, float] = {}
    for split, records in splits.items():
        start = time.perf_counter()
        if arguments.tool == "semhash":
            removals = semhash_removals(records, arguments.model_path)
        else:
            removals = univo_removals(records, arguments.tool, arguments.model_path)
        seconds[split] = time.perf_counter() - start
        curves[split] = {
            threshold: metrics(records, removals[threshold]) for threshold in THRESHOLDS
        }

    threshold = calibrate(curves["validation"])
    result: dict[str, Any] = {
        "tool": arguments.tool,
        "records": {split: len(records) for split, records in splits.items()},
        "validation_threshold": threshold,
        "test": None if threshold is None else curves["test"][threshold],
        "best_validation_f1_threshold": max(
            THRESHOLDS,
            key=lambda t: (
                2
                * curves["validation"][t]["precision"]
                * curves["validation"][t]["recall"]
                / max(
                    curves["validation"][t]["precision"] + curves["validation"][t]["recall"], 1e-12
                ),
                t,
            ),
        ),
        "seconds": seconds,
        "peak_rss_mib": peak_rss_mib(),
        "python": platform.python_version(),
        "machine": platform.machine(),
        "curves": {
            split: {str(t): point for t, point in curve.items()} for split, curve in curves.items()
        },
    }
    result["test_at_best_validation_f1"] = curves["test"][result["best_validation_f1_threshold"]]
    arguments.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: result[key] for key in result if key != "curves"}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
