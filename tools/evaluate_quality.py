"""Offline quality evaluation of the installed Rust-backed Python API.

Calibrate only on validation, evaluate once on test; train is reserved.
Run each mode/model in a fresh process to isolate model footprint and RSS.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import sys
import time
from collections import defaultdict
from pathlib import Path
from typing import TYPE_CHECKING, Any, Literal

from quality_metrics import ScoredPair, auc, choose_threshold, metrics
from quality_profiles import evaluate, summary

if TYPE_CHECKING:
    from univo import Finder


def file_hash(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def peak_rss() -> int | None:
    try:
        import resource
    except ImportError:
        return None
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (
        1 if sys.platform == "darwin" else 1024
    )


def load_pairs(path: Path) -> list[dict[str, Any]]:
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    owners: dict[str, str] = {}
    for row in rows:
        subset = row["split"]
        if subset not in {"train", "validation", "test"} or type(row["label"]) is not bool:
            raise ValueError("invalid split or label")
        if row["label"] != (row["left_entity"] == row["right_entity"]):
            raise ValueError("label conflicts with entity identifiers")
        for side in ("left", "right"):
            if not isinstance(row[side], str) or not row[side].strip():
                raise ValueError("nonempty text required")
            entity = row[f"{side}_entity"]
            if entity in owners and owners[entity] != subset:
                raise ValueError(f"entity leaks across splits: {entity}")
            owners[entity] = subset
    for subset in ("validation", "test"):
        labels = {r["label"] for r in rows if r["split"] == subset}
        if labels != {False, True}:
            raise ValueError(f"{subset} requires positive and negative pairs")
    return rows


def critical_pairs(path: Path) -> list[dict[str, Any]]:
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        case = json.loads(line)
        if case["label"] not in {"duplicate", "distinct"} or case["constraint"]:
            continue
        for view in ("short", "description"):
            if case["profile"] == "strings":
                if view != "short":
                    continue
                left, right = case["left"], case["right"]
            else:
                field = "name" if view == "short" else "description"
                left, right = case["left"].get(field), case["right"].get(field)
            if isinstance(left, str) and isinstance(right, str) and left.strip() and right.strip():
                rows.append(
                    {
                        "left": left,
                        "right": right,
                        "label": case["label"] == "duplicate",
                        "family": case["family"],
                        "lang": case["lang"],
                        "view": view,
                    }
                )
    return rows


def make_index(
    rows: list[dict[str, Any]], mode: Literal["lexical", "semantic"], model: Path | None
) -> Finder:
    from univo import Finder, Schema, Text

    finder = Finder(
        schema=Schema(id_field="id", partition_by=["pair"], fields={"text": Text(mode=mode)}),
        model_path=model,
        offline=True,
        threads=1,
        max_records=max(1, len(rows) * 2),
    )
    finder.add(
        [
            {"id": 2 * i + side, "pair": i, "text": row[name]}
            for i, row in enumerate(rows)
            for side, name in enumerate(("left", "right"))
        ]
    )
    return finder


def query(row: dict[str, Any], index: int) -> dict[str, Any]:
    return {"id": 2 * index, "pair": index, "text": row["left"]}


def scores(rows: list[dict[str, Any]], finder: Finder) -> tuple[list[ScoredPair], list[bool]]:
    scored, exact = [], []
    for index, row in enumerate(rows):
        report = finder.search(query(row, index), min_score=1e-12, limit=1, exhaustive=True)
        match = report.matches[0] if report.matches else None
        scored.append(ScoredPair(row["label"], None if match is None else match.score))
        exact.append(match is not None and match.kind == "exact")
    return scored, exact


def grouped(
    rows: list[dict[str, Any]], scored: list[ScoredPair], threshold: float | None
) -> dict[str, Any]:
    groups: dict[str, list[ScoredPair]] = defaultdict(list)
    for row, score in zip(rows, scored, strict=True):
        for key in ("lang", "family", "view", "profile"):
            if key in row:
                groups[f"{key}:{row[key]}"].append(score)
    return {
        key: {**metrics(pairs, threshold), "auc": auc(pairs)}
        for key, pairs in sorted(groups.items())
    }


def retrieval(rows: list[dict[str, Any]], finder: Finder, threshold: float) -> dict[str, Any]:
    positives = generated = final = disagreements = truncated = 0
    for index, row in enumerate(rows):
        candidate = finder.search(query(row, index), min_score=threshold, limit=1)
        oracle = finder.search(query(row, index), min_score=threshold, limit=1, exhaustive=True)
        positives += row["label"]
        generated += row["label"] and candidate.candidates > 0
        final += row["label"] and bool(candidate.matches)
        disagreements += candidate.matches != oracle.matches
        truncated += candidate.candidate_truncated or candidate.output_truncated
    return {
        "positives": positives,
        "retrieved_positive_pairs": generated,
        "candidate_recall": generated / positives if positives else None,
        "final_positive_pairs": final,
        "oracle_disagreements": disagreements,
        "truncated_queries": truncated,
        "scope": "isolated labelled pairs; one eligible target per query, not full-corpus recall",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pairs", type=Path)
    parser.add_argument("--mode", choices=["lexical", "semantic"], default="lexical")
    parser.add_argument("--model-path", type=Path)
    parser.add_argument("--critical", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.mode == "semantic" and args.model_path is None:
        parser.error("semantic evaluation requires a local model path")
    rows = load_pairs(args.pairs)
    # Score validation before even constructing the held-out test index.
    validation = [row for row in rows if row["split"] == "validation"]
    start = time.perf_counter()
    finder = make_index(validation, args.mode, args.model_path)
    validation_scores, _ = scores(validation, finder)
    threshold = choose_threshold(validation_scores)
    config = finder.config()
    if args.model_path:
        config["model"] = args.model_path.name
        config["resolved_model"] = args.model_path.name
    del finder
    test = [row for row in rows if row["split"] == "test"]
    finder = make_index(test, args.mode, args.model_path)
    test_scores, exact = scores(test, finder)
    quality = metrics(test_scores, threshold)
    result = {
        "protocol": "ror-alias-v1",
        "dataset_sha256": file_hash(args.pairs),
        "platform": platform.platform(),
        "python": platform.python_version(),
        "threads": 1,
        "config": config,
        "threshold": threshold,
        "minimum_validation_precision": 0.98,
        "minimum_validation_tp": 20,
        "validation": metrics(validation_scores, threshold),
        "test": {**quality, "auc": auc(test_scores)},
        "by_group": grouped(test, test_scores, threshold),
        "exact_baseline": metrics(
            [
                ScoredPair(p.label, 1.0 if hit else None)
                for p, hit in zip(test_scores, exact, strict=True)
            ],
            1.0,
        ),
        "retrieval": retrieval(test, finder, threshold if threshold is not None else 1.0),
        "quality_gate_passed": threshold is not None
        and float(quality["precision"] or 0) >= 0.98
        and float(quality["recall"] or 0) >= 0.90,
        "split_counts": {
            s: sum(r["split"] == s for r in rows) for s in ("train", "validation", "test")
        },
        "score_floor": 1e-12,
    }
    del finder
    if args.critical:
        profile_rows, profile_scores, failures = evaluate(args.critical, args.mode, args.model_path)
        result["critical_profiles"] = summary(profile_rows, profile_scores, failures)
        critical = critical_pairs(args.critical)
        finder = make_index(critical, args.mode, args.model_path)
        critical_scores, _ = scores(critical, finder)
        result["critical_regression"] = {
            "auc": auc(critical_scores),
            "pairs": len(critical),
            "by_group": grouped(critical, critical_scores, None),
            "scope": "text views only; no calibration, veto/partition cases excluded",
        }
    result["elapsed_seconds"] = time.perf_counter() - start
    result["peak_rss_bytes"] = peak_rss()
    if args.model_path:
        result["model_files"] = {
            name: {
                "bytes": (args.model_path / name).stat().st_size,
                "sha256": file_hash(args.model_path / name),
            }
            for name in ("config.json", "tokenizer.json", "model.safetensors")
        }
    args.output.write_text(
        json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n"
    )
    print(json.dumps({"threshold": threshold, "test": result["test"], "output": str(args.output)}))


if __name__ == "__main__":
    main()
