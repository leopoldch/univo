"""Installed-wheel contracts for pairs decided by learned weights; synthetic records."""

import random

import pytest
import univo
from univo import Exact, Finder, InvalidParameterError, Schema, Text

SYLLABLES = ["ka", "lo", "mi", "ne", "ra", "to", "vi", "su", "dor", "pel", "ban", "tri", "gon"]
CITIES = ["lyon", "paris", "lille", "nantes", "brest", "nice"]
SCHEMA = Schema(
    id_field="id",
    fields={
        "name": Text(),
        "born": Text(),
        "city": Text(),
        "sex": Exact(conflict="reject"),
    },
)


def people(persons: int) -> tuple[list[dict[str, object]], dict[str | int, int]]:
    """Persons with one to three records each: typos, missing values."""
    rng = random.Random(7)
    records: list[dict[str, object]] = []
    person_of: dict[str | int, int] = {}

    def word(syllables: int) -> str:
        return "".join(rng.choice(SYLLABLES) for _ in range(syllables))

    def typo(text: str) -> str:
        if rng.random() < 0.3:
            at = rng.randrange(len(text))
            return text[:at] + rng.choice("x7e") + text[at + 1 :]
        return text

    for person in range(persons):
        name = f"{word(2)} {word(3)}"
        born = f"{rng.randint(1800, 1950)}-{rng.randint(1, 12):02}-{rng.randint(1, 28):02}"
        city, sex = rng.choice(CITIES), rng.choice("fm")
        for _ in range(rng.randint(1, 3)):
            record: dict[str, object] = {"id": len(records), "name": typo(name), "sex": sex}
            record["born"] = typo(born) if rng.random() < 0.8 else None
            record["city"] = city if rng.random() < 0.75 else None
            person_of[len(records)] = person
            records.append(record)
    return records, person_of


def test_pairs_come_with_a_probability_and_what_each_field_weighs() -> None:
    records, person_of = people(400)
    report = Finder(schema=SCHEMA).find(records, min_probability=0.9, limit=100_000)
    assert report.total_matches == len(report.matches) > 100
    right = sum(person_of[m.left_id] == person_of[m.right_id] for m in report.matches)
    assert right >= 0.95 * len(report.matches)
    previous = float("inf")
    for match in report.matches:
        assert match.probability is not None
        assert 0.9 <= match.probability <= 1.0
        assert match.weight is not None
        assert match.weight <= previous
        previous = match.weight
        assert match.field_weights is not None
        assert list(match.field_weights) == ["name", "born", "city", "sex"]
        evidence = sum(bits for bits in match.field_weights.values() if bits is not None)
        assert evidence == pytest.approx(match.weight)
        # A field without evidence weighs nothing; a veto never comes back.
        for field, similarity in match.field_scores.items():
            assert (similarity is None) == (match.field_weights[field] is None)
        assert "sex" not in match.conflicts
    weights = report.weights
    assert isinstance(weights, univo.Weights)
    assert list(weights.fields) == ["name", "born", "city", "sex"]
    for levels in weights.fields.values():
        assert list(levels) == ["equal", "close", "partial", "different"]
    assert weights.fields["name"]["equal"] > weights.fields["city"]["equal"] > 0
    assert weights.fields["name"]["different"] < 0
    assert weights.prior < 0
    assert not report.warnings


def test_a_lower_minimum_returns_more_and_a_limit_keeps_the_most_certain() -> None:
    records, _ = people(300)
    finder = Finder(schema=SCHEMA)
    sure = finder.find(records, min_probability=0.99, limit=100_000)
    loose = finder.find(records, min_probability=0.2, limit=100_000)
    assert 0 < sure.total_matches < loose.total_matches
    assert {(m.left_id, m.right_id) for m in sure.matches} <= {
        (m.left_id, m.right_id) for m in loose.matches
    }
    few = finder.find(records, min_probability=0.2, limit=5)
    assert [(m.left_id, m.right_id) for m in few.matches] == [
        (m.left_id, m.right_id) for m in loose.matches[:5]
    ]
    assert few.total_matches == loose.total_matches
    assert few.output_truncated
    # The same records, threads or not: the same report.
    alone = Finder(schema=SCHEMA, threads=1).find(records, min_probability=0.2, limit=100_000)
    assert alone.matches == loose.matches
    assert alone.weights == loose.weights


def test_without_a_minimum_probability_nothing_changes() -> None:
    records, _ = people(50)
    report = Finder(schema=SCHEMA).find(records, min_score=0.7)
    assert report.weights is None
    assert report.matches
    for match in report.matches:
        assert match.probability is None
        assert match.weight is None
        assert match.field_weights is None


@pytest.mark.parametrize("value", [0, 0.0, -0.1, 1.5, float("nan"), "0.5", True])
def test_a_minimum_probability_outside_the_unit_interval_is_refused(value: object) -> None:
    with pytest.raises(InvalidParameterError) as error:
        Finder(schema=SCHEMA).find([], min_probability=value)  # type: ignore[arg-type]
    assert error.value.code == "parameter.invalid_option"


def test_too_few_records_give_a_report_and_a_warning() -> None:
    assert Finder(schema=SCHEMA).find([], min_probability=0.5).matches == ()
    report = Finder().find(["Amazon", "Amiazon", "Microsoft"], min_probability=0.5)
    assert report.weights is not None
    assert list(report.weights.fields) == ["<record>"]
    assert any("too few" in warning.lower() for warning in report.warnings)
