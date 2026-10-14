"""Finder.group: exact and conservative groups, never a transitive closure."""

import pathlib

import pytest
from univo import Exact, Finder, InvalidParameterError, ResourceLimitError, Schema, Text


def finder() -> Finder:
    return Finder(
        schema=Schema(
            fields={"name": Text(), "kind": Exact(conflict="reject")},
            id_field="id",
            partition_by=["tenant"],
        )
    )


def records(*rows: tuple[str, str, str]) -> list[dict[str, object]]:
    return [
        {"id": index, "tenant": tenant, "name": name, "kind": kind}
        for index, (tenant, name, kind) in enumerate(rows)
    ]


def test_exact_and_conservative_groups() -> None:
    groups = finder().group(
        records(
            ("t1", "Acme", "a"),
            ("t1", "ACME", "a"),
            ("t2", "Acme", "a"),
            ("t1", "acme", "a"),
            ("t1", "Globex", "a"),
        )
    )
    assert groups.exact == ((0, 1, 3),)
    assert [(group.members, group.min_score) for group in groups.groups] == [((0, 1, 3), 1.0)]
    assert groups.processed == 5
    assert groups.search_profile == "bounded"
    assert groups.warnings == ()


def test_a_chain_stays_open_and_is_reported() -> None:
    groups = finder().group(
        records(("t1", "abcdefgh", "a"), ("t1", "abcdefgx", "a"), ("t1", "abcdefxx", "a")),
        min_score=0.9,
    )
    assert [group.members for group in groups.groups] == [(0, 1)]
    assert (groups.qualifying_pairs, groups.split_pairs) == (2, 1)
    assert "not transitive" in groups.warnings[0]


def test_size_limit_vetoes_and_exhaustive_agreement() -> None:
    rows = records(
        ("t1", "Acme Corporation", "a"),
        ("t1", "Acme Corporatio", "a"),
        ("t1", "Acme Corporaton", "a"),
        ("t1", "Acme Corporation", "b"),
    )
    whole = finder().group(rows)
    assert [group.members for group in whole.groups] == [(0, 1, 2)]
    limited = finder().group(rows, max_group_size=2)
    assert all(len(group.members) == 2 for group in limited.groups)
    exhaustive = finder().group(rows, exhaustive=True)
    assert exhaustive.groups == whole.groups
    assert exhaustive.search_profile == "exhaustive"


def test_records_without_ids_are_named_by_position() -> None:
    groups = Finder().group(["Acme", "Globex", "ACME"])
    assert groups.exact == ((0, 2),)


def test_limits_and_parameters_are_checked() -> None:
    rows = records(*(("t1", f"Acme Corporation {i % 2}", "a") for i in range(6)))
    with pytest.raises(ResourceLimitError) as raised:
        finder().group(rows, max_pairs=0, min_score=0.5)
    assert raised.value.code == "limit.group_pairs"
    with pytest.raises(InvalidParameterError):
        finder().group(rows, min_score=1.5)
    with pytest.raises(InvalidParameterError):
        finder().group(rows, max_group_size=-1)


def test_the_documented_example_runs() -> None:
    document = pathlib.Path(__file__).resolve().parents[2] / "docs" / "guide" / "groups.md"
    text = document.read_text(encoding="utf-8")
    example = text.split("```python\n", 1)[1].split("```", 1)[0]
    exec(compile(example, str(document), "exec"), {})
