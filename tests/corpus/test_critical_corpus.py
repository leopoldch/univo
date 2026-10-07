"""Integrity of the critical corpus against docs/duplicate-definitions.md.

The oracle below re-implements the normalization and veto rules of the
definitions independently of the generator, so a mislabelled case fails here
even if the generator produced it.
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
LABELS = {"duplicate", "distinct", "compatible", "invalid"}
ERRORS = {"InvalidRecordError", "DuplicateIdError", "ResourceLimitError"}

TEXT, HOSTNAME, TYPED = "text", "hostname", "typed"
FIELD_KINDS = {
    "organizations": {
        "tenant_id": TYPED,
        "name": TEXT,
        "domain": HOSTNAME,
        "description": TEXT,
    },
    "products": {"name": TEXT, "sku": TYPED, "description": TEXT},
    "configurations": {
        "service": TYPED,
        "environment": TYPED,
        "params": TYPED,
        "description": TEXT,
    },
}
PARTITION_KEYS = {"organizations": "tenant_id"}
DISCRIMINANT_FIELDS = {"products": ("sku",), "configurations": ("service", "environment")}
MISSING = object()


def load_cases() -> list[dict[str, Any]]:
    with CORPUS.open(encoding="utf-8") as handle:
        return [json.loads(line) for line in handle]


CASES = load_cases()
VALID_CASES = [case for case in CASES if case["label"] != "invalid"]


def nfc(text: str) -> str:
    return unicodedata.normalize("NFC", text)


def normalize(kind: str, value: Any) -> Any:
    """Typed canonical form; object key order is irrelevant, list order is not."""
    if value is MISSING:
        return ("absent",)
    if value is None:
        return ("null",)
    if isinstance(value, str):
        if kind == TEXT:
            return ("str", nfc(nfc(value).casefold()))
        if kind == HOSTNAME:
            return ("str", nfc(nfc(value).casefold()).removesuffix("."))
        return ("str", nfc(value))
    if isinstance(value, dict):
        return ("dict", tuple(sorted((key, normalize(kind, item)) for key, item in value.items())))
    if isinstance(value, list):
        return ("list", tuple(normalize(kind, item) for item in value))
    return (type(value).__name__, value)


def fields(case: dict[str, Any], record: Any) -> dict[str, Any]:
    if case["profile"] == "strings":
        return {"value": record}
    return {key: record.get(key, MISSING) for key in FIELD_KINDS[case["profile"]]}


def kinds(case: dict[str, Any]) -> dict[str, str]:
    return {"value": TEXT} if case["profile"] == "strings" else FIELD_KINDS[case["profile"]]


def is_present(value: Any) -> bool:
    return value is not MISSING and value is not None


def is_informative(kind: str, value: Any) -> bool:
    if not is_present(value):
        return False
    return not (kind in (TEXT, HOSTNAME) and isinstance(value, str) and not value.strip())


def is_exact(case: dict[str, Any]) -> bool:
    field_kinds = kinds(case)
    left, right = fields(case, case["left"]), fields(case, case["right"])
    equal = all(
        normalize(kind, left[name]) == normalize(kind, right[name])
        for name, kind in field_kinds.items()
    )
    informative = any(
        is_informative(kind, left[name])
        for name, kind in field_kinds.items()
        if name != PARTITION_KEYS.get(case["profile"])
    )
    return equal and informative


def discriminant_pairs(case: dict[str, Any]) -> list[tuple[Any, Any]]:
    left, right = case["left"], case["right"]
    pairs = [
        (left.get(name, MISSING), right.get(name, MISSING))
        for name in DISCRIMINANT_FIELDS.get(case["profile"], ())
    ]
    if case["profile"] == "configurations":
        left_params, right_params = left.get("params") or {}, right.get("params") or {}
        pairs += [
            (left_params.get(name, MISSING), right_params.get(name, MISSING))
            for name in left_params.keys() | right_params.keys()
        ]
    return pairs


def has_discriminant_conflict(case: dict[str, Any]) -> bool:
    return any(
        is_present(left) and is_present(right) and normalize(TYPED, left) != normalize(TYPED, right)
        for left, right in discriminant_pairs(case)
    )


def has_one_sided_discriminant(case: dict[str, Any]) -> bool:
    return any(is_present(left) != is_present(right) for left, right in discriminant_pairs(case))


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


def test_organization_positives_include_different_domains() -> None:
    assert any(
        case["label"] == "duplicate"
        and case["profile"] == "organizations"
        and is_present(case["left"].get("domain"))
        and is_present(case["right"].get("domain"))
        and normalize(HOSTNAME, case["left"]["domain"])
        != normalize(HOSTNAME, case["right"]["domain"])
        for case in VALID_CASES
    )


def test_paraphrases_include_pairs_without_shared_identifier() -> None:
    """Pairs where only the wording can link the records isolate semantic evidence."""

    def anchor_free(case: dict[str, Any]) -> bool:
        if case["profile"] == "strings":
            return True
        left, right = case["left"], case["right"]
        anchors = ("name", "sku", "domain", "service")
        return not any(
            is_present(left.get(key)) and left.get(key) == right.get(key) for key in anchors
        )

    paraphrases = [case for case in CASES if case["family"] == "paraphrased_description"]
    labels = Counter(case["label"] for case in paraphrases if anchor_free(case))
    assert labels["duplicate"] >= 4
    assert labels["distinct"] >= 4


@pytest.mark.parametrize("case", CASES, ids=lambda case: case["case_id"])
def test_case_metadata_is_consistent(case: dict[str, Any]) -> None:
    assert case["label"] in LABELS
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
            assert set(record) <= {"id", *FIELD_KINDS[case["profile"]]}
            assert isinstance(record["id"], str)
    if case["profile"] != "strings":
        assert case["left"]["id"] != case["right"]["id"]


@pytest.mark.parametrize("case", VALID_CASES, ids=lambda case: case["case_id"])
def test_exact_label_matches_field_aware_normalization(case: dict[str, Any]) -> None:
    """A pair is labelled exact if and only if the oracle finds it exact."""
    assert is_exact(case) == (case["expected_kind"] == "exact")


@pytest.mark.parametrize("case", VALID_CASES, ids=lambda case: case["case_id"])
def test_veto_label_matches_discriminant_conflicts(case: dict[str, Any]) -> None:
    """A discriminant conflict always vetoes, and a veto always has one."""
    assert has_discriminant_conflict(case) == (case["constraint"] == "veto")


@pytest.mark.parametrize(
    "case",
    [case for case in CASES if case["label"] == "compatible"],
    ids=lambda case: case["case_id"],
)
def test_compatible_cases_lack_evidence_without_contradiction(case: dict[str, Any]) -> None:
    assert not has_discriminant_conflict(case)
    assert has_one_sided_discriminant(case)


@pytest.mark.parametrize(
    "case",
    [
        case
        for case in VALID_CASES
        if case["profile"] == "configurations" and case["label"] == "duplicate"
    ],
    ids=lambda case: case["case_id"],
)
def test_configuration_duplicates_have_complete_parameters(case: dict[str, Any]) -> None:
    """Configurations require equal parameter values; one-sided ones are only compatible."""
    assert not has_one_sided_discriminant(case)


@pytest.mark.parametrize(
    "case",
    [case for case in VALID_CASES if case["profile"] == "organizations"],
    ids=lambda case: case["case_id"],
)
def test_same_tenant_unless_partition_case(case: dict[str, Any]) -> None:
    same = normalize(TYPED, case["left"]["tenant_id"]) == normalize(
        TYPED, case["right"]["tenant_id"]
    )
    assert same == (case["constraint"] != "partition")
