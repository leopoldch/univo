"""Explicit HNSW routing, observed bounds and snapshot compatibility."""

import hashlib
import json
import pathlib
from typing import Any
from unittest.mock import patch

import pytest
from univo import Exact, Finder, IndexCompatibilityError, InvalidParameterError, Schema, Text

TINY = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "models" / "tiny"


def finder(**options: Any) -> Finder:
    return Finder(
        schema=Schema(
            id_field="id",
            partition_by=["tenant"],
            fields={
                "text": Text(mode="semantic"),
                "data": Exact(weight=0.01),
            },
        ),
        model_path=TINY,
        offline=True,
        max_records=10001,
        **options,
    )


def records() -> list[dict[str, object]]:
    return [
        {"id": i, "tenant": "a", "text": "amazon" if i % 2 else "microsoft", "data": i}
        for i in range(10000)
    ]


def test_approximation_saturation_mutations_and_exhaustive_override() -> None:
    index = finder(retrieval="hnsw", ann_max_candidates=8)
    index.add(records())
    assert index.stats()["ann_nodes"] == 2
    query = {"id": -1, "tenant": "a", "text": "amazon", "data": -1}
    report = index.search(query, min_score=0.9, limit=100)
    oracle = index.search(query, min_score=0.9, limit=10000, exhaustive=True)
    assert report.search_profile == "approximate"
    assert report.candidate_truncated
    assert report.total_matches == 8
    assert len(report.warnings) == 2
    assert all(match in oracle.matches for match in report.matches)
    assert oracle.search_profile == "exhaustive"
    assert not oracle.candidate_truncated
    deleted = [match.right_id for match in report.matches]
    assert index.remove(deleted) == []
    after = index.search(query, min_score=0.9, limit=100)
    assert not {match.right_id for match in after.matches}.intersection(deleted)
    index.add([{"id": "rare", "tenant": "b", "text": "amazon", "data": -1}])
    rare = index.search({**query, "tenant": "b"})
    assert rare.search_profile == "bounded"
    assert [match.right_id for match in rare.matches] == ["rare"]


def test_snapshot_preserves_parameters_and_refuses_tampered_configuration(
    tmp_path: pathlib.Path,
) -> None:
    index = finder(retrieval="hnsw", ann_max_candidates=8)
    index.add(records())
    query = {"id": -1, "tenant": "a", "text": "amazon", "data": -1}
    before = index.search(query, min_score=0.9, limit=100)
    path = tmp_path / "index"
    index.save(path)
    with patch("urllib.request.urlopen", side_effect=AssertionError("network")):
        restored = Finder.load(path, offline=True)
    assert restored.config()["retrieval"] == "hnsw"
    assert restored.config()["ann_max_candidates"] == 8
    assert restored.search(query, min_score=0.9, limit=100) == before
    assert restored.stats()["ann_nodes"] == 2
    generation = path / (path / "CURRENT").read_text().splitlines()[0]
    manifest = generation / "manifest.json"
    contents = json.loads(manifest.read_text())
    contents["configuration"]["ann_max_candidates"] = 16
    encoded = json.dumps(contents).encode()
    manifest.write_bytes(encoded)
    checksum = hashlib.sha256(encoded).hexdigest()
    (generation / "COMPLETE").write_text(checksum)
    (path / "CURRENT").write_text(f"{generation.name}\n{checksum}\n")
    with pytest.raises(IndexCompatibilityError, match="ANN configuration changed"):
        Finder.load(path, offline=True)


def test_default_exact_and_small_partition_fallback_allocate_no_graph() -> None:
    dense = finder()
    fallback = finder(retrieval="hnsw")
    rows = records()[:100]
    dense.add(rows)
    fallback.add(rows)
    query = {"id": -1, "tenant": "a", "text": "amazon", "data": -1}
    assert dense.search(query).matches == fallback.search(query).matches
    assert fallback.search(query).search_profile == "bounded"
    assert dense.stats()["ann_bytes"] == fallback.stats()["ann_bytes"] == 0
    assert dense.config()["config_fingerprint"] != fallback.config()["config_fingerprint"]
    lexical = Finder(retrieval="hnsw", offline=True)
    with patch("univo.models._load", side_effect=AssertionError("model loaded")):
        lexical.add(["amazon"])
        assert lexical.stats()["ann_nodes"] == 0


@pytest.mark.parametrize(
    "options",
    [
        {"retrieval": "auto"},
        {"retrieval": None},
        {"ann_ef_search": 0},
        {"ann_ef_search": 8193},
        {"ann_ef_search": True},
        {"ann_max_candidates": 0},
        {"ann_max_candidates": 100001},
        {"ann_max_candidates": 1.5},
    ],
)
def test_invalid_ann_options_fail_before_model_resolution(options: dict[str, Any]) -> None:
    with (
        patch("univo.models._load", side_effect=AssertionError("model loaded")),
        pytest.raises(InvalidParameterError),
    ):
        finder(**options)
