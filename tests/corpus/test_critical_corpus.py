"""Integrity of the critical corpus against docs/duplicate-definitions.md.

These checks re-derive structural facts from the records themselves, so a
mislabelled case fails here even if the generator produced it.
"""

import json
import pathlib
import re
import unicodedata
from collections import Counter
from typing import Any

import pytest

CORPUS = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "critical" / "cases.jsonl"

FAMILIES = {
    "typo",
    "semantic_competitor",
    "paraphrased_description",
    "homonym",
    "negation",
    "critical_parameter",
    "version_environment",
    "missing_value",
    "unicode",
    "ordering",
    "tenant",
    "dense_series",
    "short_string",
    "adversarial",
}
PROFILE_FIELDS = {
    "organizations": {"id", "tenant_id", "name", "domain", "description"},
    "products": {"id", "name", "sku", "description"},
    "configurations": {"id", "service", "environment", "params", "description"},
}
DISCRIMINANT_FIELDS = {
    "products": ("sku",),
    "configurations": ("service", "environment"),
}
ERRORS = {"InvalidRecordError", "DuplicateIdError", "ResourceLimitError"}
MISSING = object()


def load_cases() -> list[dict[str, Any]]:
    with CORPUS.open(encoding="utf-8") as handle:
        return [json.loads(line) for line in handle]


CASES = load_cases()
VALID_CASES = [case for case in CASES if case["label"] != "invalid"]


def canonical(value: Any) -> Any:
    """Typed, order-insensitive form for objects, NFC for text."""
    if isinstance(value, str):
        return ("str", unicodedata.normalize("NFC", value))
    if isinstance(value, dict):
        return ("dict", tuple(sorted((key, canonical(item)) for key, item in value.items())))
    if isinstance(value, list):
        return ("list", tuple(canonical(item) for item in value))
    return (type(value).__name__, value)


def is_present(value: Any) -> bool:
    return value is not MISSING and value is not None


def present_conflicts(left: Any, right: Any) -> bool:
    return is_present(left) and is_present(right) and canonical(left) != canonical(right)


def has_discriminant_conflict(case: dict[str, Any]) -> bool:
    left, right = case["left"], case["right"]
    for field in DISCRIMINANT_FIELDS.get(case["profile"], ()):
        if present_conflicts(left.get(field, MISSING), right.get(field, MISSING)):
            return True
    if case["profile"] == "configurations":
        left_params, right_params = left.get("params") or {}, right.get("params") or {}
        for name in left_params.keys() & right_params.keys():
            if present_conflicts(left_params[name], right_params[name]):
                return True
    return False


def without_id(record: Any) -> Any:
    if isinstance(record, dict):
        return {key: value for key, value in record.items() if key != "id"}
    return record


def test_corpus_size_and_coverage() -> None:
    assert len(CASES) >= 200
    families = Counter(case["family"] for case in CASES)
    assert set(families) == FAMILIES
    assert min(families.values()) >= 10


def test_case_identifiers_are_unique_and_derived_from_family() -> None:
    identifiers = [case["case_id"] for case in CASES]
    assert len(identifiers) == len(set(identifiers))
    for case in CASES:
        assert re.fullmatch(rf"{case['family']}-\d{{3}}", case["case_id"])


def test_french_cases_are_numerous_enough_for_a_separate_report() -> None:
    french = [case for case in CASES if case["lang"] == "fr"]
    assert len(french) >= 40
    assert {case["label"] for case in french} >= {"duplicate", "distinct"}


@pytest.mark.parametrize("case", CASES, ids=lambda case: case["case_id"])
def test_case_metadata_is_consistent(case: dict[str, Any]) -> None:
    assert case["label"] in {"duplicate", "distinct", "invalid"}
    assert case["rationale"].strip()
    assert (case["label"] == "invalid") == (case["expected_error"] is not None)
    if case["expected_error"] is not None:
        assert case["expected_error"] in ERRORS
    if case["constraint"] is not None:
        assert case["constraint"] in {"veto", "partition"}
        assert case["label"] == "distinct"
    if case["expected_kind"] is not None:
        assert case["expected_kind"] == "exact"
        assert case["label"] == "duplicate"


@pytest.mark.parametrize("case", VALID_CASES, ids=lambda case: case["case_id"])
def test_valid_records_match_their_profile(case: dict[str, Any]) -> None:
    for record in (case["left"], case["right"]):
        if case["profile"] == "strings":
            assert isinstance(record, str)
        else:
            assert isinstance(record, dict)
            assert set(record) <= PROFILE_FIELDS[case["profile"]]
            assert isinstance(record["id"], str)
    if case["profile"] != "strings":
        assert case["left"]["id"] != case["right"]["id"]


@pytest.mark.parametrize(
    "case",
    [case for case in CASES if case["expected_kind"] == "exact"],
    ids=lambda case: case["case_id"],
)
def test_exact_cases_are_equal_after_normalization(case: dict[str, Any]) -> None:
    assert canonical(without_id(case["left"])) == canonical(without_id(case["right"]))


@pytest.mark.parametrize(
    "case",
    [case for case in CASES if case["constraint"] == "partition"],
    ids=lambda case: case["case_id"],
)
def test_partition_cases_have_different_partition_keys(case: dict[str, Any]) -> None:
    assert canonical(case["left"]["tenant_id"]) != canonical(case["right"]["tenant_id"])


@pytest.mark.parametrize("case", VALID_CASES, ids=lambda case: case["case_id"])
def test_veto_label_matches_discriminant_conflicts(case: dict[str, Any]) -> None:
    """A discriminant conflict always vetoes, and a veto always has one."""
    assert has_discriminant_conflict(case) == (case["constraint"] == "veto")


@pytest.mark.parametrize(
    "case",
    [case for case in VALID_CASES if case["profile"] == "organizations"],
    ids=lambda case: case["case_id"],
)
def test_same_tenant_unless_partition_case(case: dict[str, Any]) -> None:
    same = canonical(case["left"]["tenant_id"]) == canonical(case["right"]["tenant_id"])
    assert same == (case["constraint"] != "partition")
