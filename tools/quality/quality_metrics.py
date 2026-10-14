"""Metrics only: record similarity and candidate retrieval remain in Rust."""

import math
from collections.abc import Sequence
from dataclasses import dataclass


@dataclass(frozen=True)
class ScoredPair:
    label: bool
    score: float | None


def metrics(pairs: Sequence[ScoredPair], threshold: float | None) -> dict[str, int | float | None]:
    predicted = [
        threshold is not None and p.score is not None and p.score >= threshold for p in pairs
    ]
    tp = sum(p.label and hit for p, hit in zip(pairs, predicted, strict=True))
    fp = sum(not p.label and hit for p, hit in zip(pairs, predicted, strict=True))
    positives = sum(p.label for p in pairs)
    precision = tp / (tp + fp) if tp + fp else None
    recall = tp / positives if positives else None
    return {
        "pairs": len(pairs),
        "positives": positives,
        "tp": tp,
        "fp": fp,
        "fn": positives - tp,
        "tn": len(pairs) - positives - fp,
        "precision": precision,
        "recall": recall,
        "f1": 2 * tp / (2 * tp + fp + positives - tp) if positives + fp else None,
        "no_returned_score": sum(p.score is None for p in pairs),
        "precision_lower_95": wilson_lower(tp, tp + fp),
    }


def wilson_lower(successes: int, total: int) -> float | None:
    if not total:
        return None
    z = 1.959963984540054
    p = successes / total
    return (
        p + z * z / (2 * total) - z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total))
    ) / (1 + z * z / total)


def auc(pairs: Sequence[ScoredPair]) -> float | None:
    """Rank AUC; unavailable evidence ranks below all positive scores."""
    ordered = sorted(pairs, key=lambda p: -1.0 if p.score is None else p.score)
    positives = sum(p.label for p in pairs)
    negatives = len(pairs) - positives
    if not positives or not negatives:
        return None
    wins = 0.0
    negatives_before = 0
    start = 0
    while start < len(ordered):
        end = start + 1
        while end < len(ordered) and ordered[end].score == ordered[start].score:
            end += 1
        group_positives = sum(p.label for p in ordered[start:end])
        group_negatives = end - start - group_positives
        wins += group_positives * (negatives_before + group_negatives / 2)
        negatives_before += group_negatives
        start = end
    return wins / (positives * negatives)


def choose_threshold(
    validation: Sequence[ScoredPair], minimum_precision: float = 0.98, minimum_tp: int = 20
) -> float | None:
    """Maximum validation recall at the declared precision, or abstention.

    Process each tied score together. Higher thresholds win recall ties.
    This is empirical calibration, not a production confidence guarantee.
    """
    ordered = sorted(
        (p for p in validation if p.score is not None), key=lambda p: -float(p.score or 0)
    )
    tp = fp = 0
    best_tp = -1
    chosen = None
    start = 0
    while start < len(ordered):
        score = ordered[start].score
        end = start
        while end < len(ordered) and ordered[end].score == score:
            tp += ordered[end].label
            fp += not ordered[end].label
            end += 1
        if tp >= minimum_tp and tp / (tp + fp) >= minimum_precision and tp > best_tp:
            chosen, best_tp = score, tp
        start = end
    return chosen
