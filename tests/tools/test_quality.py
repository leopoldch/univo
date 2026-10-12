"""Independent metric oracles and entity leakage checks for U-012."""

import json
import random
from pathlib import Path
from typing import Any

import pytest
from build_quality_corpus import build, split
from evaluate_quality import load_pairs
from quality_metrics import ScoredPair, auc, choose_threshold, metrics, wilson_lower


def test_auc_matches_pairwise_oracle_with_ties_and_missing_scores() -> None:
    rng = random.Random(12)
    for _ in range(30):
        pairs = [
            ScoredPair(bool(rng.randrange(2)), rng.choice([None, 0.1, 0.5, 1.0])) for _ in range(40)
        ]
        positives = [-1 if p.score is None else p.score for p in pairs if p.label]
        negatives = [-1 if p.score is None else p.score for p in pairs if not p.label]
        wins = sum(float(p > n) + 0.5 * (p == n) for p in positives for n in negatives)
        assert auc(pairs) == wins / (len(positives) * len(negatives))
    assert auc([ScoredPair(True, 1)]) is None


def test_threshold_uses_whole_ties_and_abstains_when_precision_is_unreachable() -> None:
    pairs = [ScoredPair(True, 0.9)] * 20 + [ScoredPair(False, 0.8)] + [ScoredPair(True, 0.8)]
    assert choose_threshold(pairs) == 0.9
    assert choose_threshold(pairs, minimum_precision=0.95) == 0.8
    assert choose_threshold([ScoredPair(True, 1)] * 19) is None
    assert choose_threshold([ScoredPair(True, 1), ScoredPair(False, 1)], minimum_tp=1) is None


def test_report_counts_missing_evidence_as_a_missed_duplicate() -> None:
    report = metrics([ScoredPair(True, 1), ScoredPair(True, None), ScoredPair(False, 1)], 1)
    assert (report["tp"], report["fp"], report["fn"], report["tn"]) == (1, 1, 1, 0)
    assert report["precision"] == report["recall"] == report["f1"] == 0.5
    assert metrics([ScoredPair(True, 1)], None)["precision"] is None
    assert wilson_lower(0, 0) is None
    lower = wilson_lower(20, 20)
    assert lower is not None
    assert lower < 0.85  # 20/20 cannot establish 99.5% precision.


def source(entity: str) -> dict[str, Any]:
    return {
        "id": entity,
        "status": "active",
        "names": [
            {"value": f"Research {entity}", "lang": "en", "types": ["ror_display"]},
            {"value": f"Institut {entity}", "lang": "fr", "types": ["label"]},
        ],
    }


def test_builder_preserves_entity_groups_and_uses_only_source_aliases(tmp_path: Path) -> None:
    sources = [source(f"ror-{i}") for i in range(100)]
    rows = build(sources, 100)
    assert rows == build(list(reversed(sources)), 100)
    assert len([r for r in rows if r["label"]]) == 100
    for row in rows:
        assert split(row["left_entity"]) == split(row["right_entity"]) == row["split"]
        assert row["label"] == (row["left_entity"] == row["right_entity"])
    path = tmp_path / "pairs.jsonl"
    path.write_text("".join(json.dumps(row) + "\n" for row in rows))
    assert load_pairs(path) == rows
    rows[0]["split"] = "test" if rows[0]["split"] != "test" else "validation"
    path.write_text("".join(json.dumps(row) + "\n" for row in rows))
    with pytest.raises(ValueError, match="leaks"):
        load_pairs(path)


def test_loader_rejects_labels_inconsistent_with_entities(tmp_path: Path) -> None:
    rows = build([source(f"ror-{i}") for i in range(100)], 100)
    rows[0]["label"] = not rows[0]["label"]
    path = tmp_path / "pairs.jsonl"
    path.write_text("".join(json.dumps(row) + "\n" for row in rows))
    with pytest.raises(ValueError, match="conflicts"):
        load_pairs(path)
