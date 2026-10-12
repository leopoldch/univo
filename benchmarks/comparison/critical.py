"""U-003: hard rules and ranking on the critical corpus, Univo against SemHash.

The 234 critical cases (tests/fixtures/critical) are structured pairs:
organizations partitioned by tenant, products with a discriminant SKU,
configurations whose service, environment and parameters must agree. 44
pairs must never match: 32 contradict a discriminant field, 12 sit in
different partitions. Univo is given the schemas of the U-012 profiles;
SemHash is given the same fields as columns, which it embeds, with missing
values as empty strings and objects as JSON, since it refuses None.

The corpus never chooses a threshold: results are reported at each tool's
default (Univo 0.8, SemHash 0.9) and over the same sweep as dedupe.py.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Any, Literal

THRESHOLDS = [round(0.5 + step / 100, 2) for step in range(51)]
DEFAULTS = {"univo-lexical": 0.8, "univo-semantic": 0.8, "semhash": 0.9}
COLUMNS = {
    "strings": None,
    "organizations": ["name", "domain", "description", "tenant_id"],
    "products": ["name", "sku", "description"],
    "configurations": ["service", "environment", "params", "description"],
}


def cases(path: Path) -> list[dict[str, Any]]:
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    return [row for row in rows if row["label"] in ("duplicate", "distinct")]


def univo_scores(rows: list[dict[str, Any]], mode: str, model: str | None) -> list[float | None]:
    sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
    from quality_profiles import schema
    from univo import Finder

    text_mode: Literal["lexical", "semantic"] = (
        "semantic" if mode == "univo-semantic" else "lexical"
    )
    finders = {
        profile: Finder(
            schema=schema(profile, text_mode),
            model_path=model if text_mode == "semantic" else None,
            offline=True,
        )
        for profile in COLUMNS
    }
    scores: list[float | None] = []
    for row in rows:
        report = finders[row["profile"]].find(
            [row["left"], row["right"]], min_score=1e-12, limit=1, exhaustive=True
        )
        # No match: another partition, a veto, or no evidence.
        scores.append(report.matches[0].score if report.matches else None)
    return scores


def as_text(value: object) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    return json.dumps(value, sort_keys=True, ensure_ascii=False)


def semhash_scores(rows: list[dict[str, Any]], model: str) -> list[float | None]:
    from model2vec import StaticModel
    from semhash import SemHash

    encoder = StaticModel.from_pretrained(model)
    scores: list[float | None] = []
    for row in rows:
        columns = COLUMNS[row["profile"]]
        if columns is None:
            records = [{"text": as_text(row["left"])}, {"text": as_text(row["right"])}]
            columns = ["text"]
        else:
            records = [
                {column: as_text(side.get(column)) for column in columns}
                for side in (row["left"], row["right"])
            ]
        result = SemHash.from_records(records, columns=columns, model=encoder).self_deduplicate(
            threshold=-1.0
        )
        scores.append(result.filtered[0].score if result.filtered else None)
    return scores


def at(rows: list[dict[str, Any]], scores: list[float | None], threshold: float) -> dict[str, int]:
    matched = [score is not None and score >= threshold for score in scores]

    def count(test: Callable[[dict[str, Any]], bool]) -> int:
        return sum(m for m, row in zip(matched, rows, strict=True) if test(row))

    return {
        "hard_rule_violations": count(lambda row: row["constraint"] in ("veto", "partition")),
        "veto_violations": count(lambda row: row["constraint"] == "veto"),
        "partition_violations": count(lambda row: row["constraint"] == "partition"),
        "duplicates_found": count(lambda row: row["label"] == "duplicate"),
        "distinct_matched": count(lambda row: row["label"] == "distinct" and not row["constraint"]),
    }


def auc(rows: list[dict[str, Any]], scores: list[float | None]) -> float:
    """Unconstrained duplicates against distinct pairs; no score ranks last."""

    def value(score: float | None) -> float:
        return -2.0 if score is None else score

    positives = [value(s) for s, r in zip(scores, rows, strict=True) if r["label"] == "duplicate"]
    negatives = [
        value(s)
        for s, r in zip(scores, rows, strict=True)
        if r["label"] == "distinct" and not r["constraint"]
    ]
    wins = sum((p > n) + 0.5 * (p == n) for p in positives for n in negatives)
    return wins / (len(positives) * len(negatives))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cases", type=Path, help="tests/fixtures/critical/cases.jsonl")
    parser.add_argument("--tool", choices=tuple(DEFAULTS), required=True)
    parser.add_argument("--model-path")
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args(argv)
    if arguments.tool != "univo-lexical" and not arguments.model_path:
        parser.error("--model-path is required for semantic tools")

    rows = cases(arguments.cases)
    if arguments.tool == "semhash":
        scores = semhash_scores(rows, arguments.model_path)
    else:
        scores = univo_scores(rows, arguments.tool, arguments.model_path)
    sweep = {str(threshold): at(rows, scores, threshold) for threshold in THRESHOLDS}
    result = {
        "tool": arguments.tool,
        "pairs": len(rows),
        "must_never_match": sum(row["constraint"] in ("veto", "partition") for row in rows),
        "duplicates": sum(row["label"] == "duplicate" for row in rows),
        "distinct_unconstrained": sum(
            row["label"] == "distinct" and not row["constraint"] for row in rows
        ),
        "default_threshold": DEFAULTS[arguments.tool],
        "at_default": at(rows, scores, DEFAULTS[arguments.tool]),
        "auc": auc(rows, scores),
        "sweep": sweep,
    }
    arguments.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: result[key] for key in result if key != "sweep"}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
