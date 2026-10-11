"""Critical profile regression through Finder; no score implementation here."""

from __future__ import annotations

import json
from pathlib import Path
from typing import TYPE_CHECKING, Any, Literal

from quality_metrics import ScoredPair, auc

if TYPE_CHECKING:
    from univo import Schema


def schema(profile: str, mode: Literal["lexical", "semantic"]) -> Schema | None:
    from univo import Exact, Schema, Text

    if profile == "strings":
        return None  # Generic strings stay lexical in SCORING_VERSION=2.
    fields: dict[str, Text | Exact] = {"description": Text(mode=mode)}
    partitions: list[str] = []
    if profile == "organizations":
        fields.update(name=Text(), domain=Exact(normalizer="hostname"))
        partitions = ["tenant_id"]
    elif profile == "products":
        fields.update(name=Text(), sku=Exact(conflict="reject"))
    elif profile == "configurations":
        fields.update(
            {key: Exact(conflict="reject") for key in ("service", "environment", "params")}
        )
    else:
        raise ValueError(f"unknown profile: {profile}")
    return Schema(fields=fields, id_field="id", partition_by=partitions)


def evaluate(
    path: Path, mode: Literal["lexical", "semantic"], model: Path | None
) -> tuple[list[dict[str, Any]], list[ScoredPair], list[str]]:
    from univo import Finder
    from univo.errors import UnivoError

    cases = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    rows, scored, failures = [], [], []
    for profile in sorted({case["profile"] for case in cases}):
        finder: Finder = Finder(schema=schema(profile, mode), model_path=model, offline=True)
        for case in cases:
            if case["profile"] != profile:
                continue
            try:
                report = finder.find(
                    [case["left"], case["right"]], min_score=1e-12, limit=1, exhaustive=True
                )
            except UnivoError as error:
                if case["label"] != "invalid" or type(error).__name__ != case["expected_error"]:
                    failures.append(case["case_id"])
                continue
            match = report.matches[0] if report.matches else None
            expected_exact = case["expected_kind"] == "exact"
            if (match is not None and match.kind == "exact") != expected_exact:
                failures.append(case["case_id"])
            if case["label"] == "invalid" or (case["constraint"] and match is not None):
                failures.append(case["case_id"])
            if case["label"] in {"duplicate", "distinct"} and not case["constraint"]:
                rows.append(case)
                scored.append(
                    ScoredPair(case["label"] == "duplicate", None if match is None else match.score)
                )
        del finder  # At most one profile's model is resident.
    return rows, scored, failures


def summary(
    rows: list[dict[str, Any]], pairs: list[ScoredPair], failures: list[str]
) -> dict[str, Any]:
    languages = sorted({r["lang"] for r in rows})
    return {
        "pairs": len(pairs),
        "auc": auc(pairs),
        "hard_gate_failures": failures,
        "by_language": {
            lang: {
                "pairs": sum(r["lang"] == lang for r in rows),
                "auc": auc([p for r, p in zip(rows, pairs, strict=True) if r["lang"] == lang]),
            }
            for lang in languages
        },
        "scope": "full profile scores; constraints checked but excluded from AUC; no tuning",
    }
