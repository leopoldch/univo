"""Installed-wheel contracts for Finder; all records and models are synthetic."""

import copy
import pathlib
from concurrent.futures import ThreadPoolExecutor

import pytest
from univo import (
    DuplicateIdError,
    Exact,
    Finder,
    InvalidParameterError,
    InvalidRecordError,
    InvalidSchemaError,
    ModelUnavailableError,
    ResourceLimitError,
    Schema,
    Text,
)

TINY = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "models" / "tiny"


def test_quickstart_and_temporary_index() -> None:
    finder = Finder(offline=True)
    report = finder.find(["Amazon", "Amiazon", "Microsoft"])
    assert [(m.left_id, m.right_id) for m in report.matches] == [(0, 1)]
    assert report.matches[0].kind == "candidate"
    assert not finder.config()["model_loaded"]
    assert finder.stats()["records"] == 0
    assert finder.add(["Amazon"]) == [0]
    assert finder.add(["Microsoft"]) == [1]
    finder.find(["Other", "other"])
    assert finder.stats()["records"] == 2
    assert finder.search("Amazon").matches[0].right_id == 0


def test_partitions_vetoes_ids_and_input_preservation() -> None:
    schema = Schema(
        id_field="id",
        partition_by=["tenant"],
        fields={
            "name": Text(),
            "env": Exact(conflict="reject"),
        },
    )
    records = [
        {"id": "a", "tenant": "x", "name": "Amazon", "env": "prod"},
        {"id": "b", "tenant": "x", "name": "AMAZON", "env": "prod"},
        {"id": "c", "tenant": "y", "name": "Amazon", "env": "prod"},
        {"id": "d", "tenant": "x", "name": "Amazon", "env": "test"},
    ]
    original = copy.deepcopy(records)
    finder = Finder(schema=schema, offline=True)
    assert finder.add(records) == ["a", "b", "c", "d"]
    report = finder.find(records)
    assert [(m.left_id, m.right_id) for m in report.matches] == [("a", "b")]
    assert report.matches[0].kind == "exact"
    assert any("case" in reason for reason in report.matches[0].reasons)
    assert [m.right_id for m in finder.search(records[0]).matches] == ["b"]
    assert finder.stats()["generation"] == report.generation == 1
    assert records == original


def test_add_is_atomic_and_errors_carry_batch_location() -> None:
    finder = Finder(fields=["name"], id_field="id")
    finder.add([{"id": 1, "name": "Amazon"}])
    before = finder.stats()
    with pytest.raises(InvalidRecordError) as failure:
        finder.add([{"id": 2, "name": "Amazon"}, {"id": 3, "name": object()}])
    assert failure.value.code == "record.unsupported_type"
    assert failure.value.path == "name"
    assert failure.value.record_index == 1
    assert finder.stats() == before
    with pytest.raises(DuplicateIdError):
        finder.add([{"id": 2, "name": "Amazon"}, {"id": 1, "name": "Amazon"}])
    assert finder.stats() == before
    assert finder.add([{"id": "1", "name": "Amazon"}]) == ["1"]


def test_only_selected_values_are_converted() -> None:
    finder = Finder(fields=["nested.name"])
    row = {"nested": {"name": "Amazon", "ignored": object()}, "ignored": object()}
    assert finder.add([row]) == [0]
    assert finder.search(row).matches[0].kind == "exact"


@pytest.mark.parametrize(
    ("value", "code"),
    [
        (float("nan"), "record.non_finite_number"),
        (2**100, "record.integer_out_of_range"),
        (object(), "record.unsupported_type"),
    ],
)
def test_selected_typed_values_fail(value: object, code: str) -> None:
    finder = Finder(schema=Schema(fields={"v": Exact()}))
    with pytest.raises(InvalidRecordError) as failure:
        finder.add([{"v": value}])
    assert failure.value.code == code
    assert finder.stats()["records"] == 0


def test_typed_exact_and_missing_evidence() -> None:
    finder = Finder(schema=Schema(fields={"v": Exact()}))
    report = finder.find([{"v": True}, {"v": 1}, {"v": "1"}, {}, {}])
    assert report.matches == ()
    assert finder.find([{}, {}], exhaustive=True).matches == ()


def test_output_limit_and_oracle_agree() -> None:
    finder = Finder()
    records = ["Amazon", "Amazon", "Amazon", "Amiazon", "Microsoft"]
    union = finder.find(records, limit=2)
    oracle = finder.find(records, limit=2, exhaustive=True)
    assert union.matches == oracle.matches
    assert union.total_matches == oracle.total_matches == 6
    assert union.output_truncated
    assert not union.candidate_truncated
    assert union.search_profile == "bounded"
    assert oracle.search_profile == "exhaustive"
    assert oracle.candidates == len(records) * (len(records) - 1) // 2
    assert finder.find(records, limit=0).matches == ()
    assert finder.find(records, limit=0).total_matches == 6


def test_candidate_work_limit_is_observable() -> None:
    finder = Finder()
    finder.add(["Amazon", "Amazon", "Amiazon"])
    report = finder.search("Amazon", max_entries=0)
    assert report.candidate_truncated
    assert not report.output_truncated
    assert len(report.matches) == 2  # Exact lane is never work-limited.
    assert report.warnings


def test_resource_limits_and_cycles_are_bounded() -> None:
    finder = Finder(schema=Schema(fields={"v": Exact()}, max_depth=3), max_records=2)
    cycle: list[object] = []
    cycle.append(cycle)
    with pytest.raises(ResourceLimitError) as failure:
        finder.add([{"v": cycle}])
    assert failure.value.code == "limit.nesting_depth"
    assert finder.stats()["records"] == 0
    with pytest.raises(ResourceLimitError):
        finder.add({"v": n} for n in range(3))
    assert finder.stats()["records"] == 0
    small = Finder(schema=Schema(fields={"v": Exact()}, max_selected_bytes=64))
    with pytest.raises(ResourceLimitError):
        small.add([{"v": "x" * 65}])


def test_schema_and_options_fail_early() -> None:
    with pytest.raises(InvalidSchemaError):
        Schema(fields={})
    with pytest.raises(InvalidSchemaError):
        Schema(fields={"name": Text(weight=0)})
    with pytest.raises(InvalidParameterError):
        Finder(threads=0)
    with pytest.raises(InvalidParameterError):
        Finder(model="default", model_path=TINY)
    with pytest.raises(InvalidParameterError):
        Schema(fields={"name": Text(mode="bad")})  # type: ignore[arg-type]
    for minimum in (0.0, float("nan"), 2.0):
        with pytest.raises(InvalidParameterError):
            Finder().find([], min_score=minimum)
    with pytest.raises(InvalidParameterError):
        Finder().search("Amazon", limit=-1)


def test_lazy_local_model_and_thread_budget() -> None:
    finder = Finder(
        schema=Schema(fields={"text": Text(mode="semantic")}),
        model_path=TINY,
        offline=True,
        threads=2,
    )
    assert not finder.config()["model_loaded"]
    assert finder.add([{"text": "online retail"}, {"text": "online retail"}]) == [0, 1]
    assert finder.config()["model_loaded"]
    assert finder.config()["effective_threads"] == 2
    assert finder.search({"text": "online retail"}).matches[0].score == pytest.approx(1)


def test_offline_model_failure_leaves_no_records(tmp_path: pathlib.Path) -> None:
    finder = Finder(
        schema=Schema(fields={"text": Text(mode="semantic")}), offline=True, cache_dir=tmp_path
    )
    with pytest.raises(ModelUnavailableError):
        finder.add([{"text": "hello"}])
    assert finder.stats()["records"] == 0


def test_concurrent_batches_are_serialized() -> None:
    finder = Finder(fields=["name"], id_field="id")
    with ThreadPoolExecutor(max_workers=4) as pool:
        results = list(pool.map(lambda i: finder.add([{"id": i, "name": "Amazon"}]), range(20)))
    assert results == [[i] for i in range(20)]
    assert finder.stats()["records"] == finder.stats()["generation"] == 20


def test_exhaustive_pair_budget_and_partition_allowance() -> None:
    finder = Finder(max_exhaustive_pairs=2)
    with pytest.raises(ResourceLimitError) as failure:
        finder.find(["Amazon"] * 3, exhaustive=True)
    assert failure.value.code == "limit.pair_count"
    assert finder.find(["Amazon"] * 3).total_matches == 3
    partitioned = Finder(
        schema=Schema(partition_by=["tenant"], fields={"name": Text()}), max_exhaustive_pairs=0
    )
    assert (
        partitioned.find(
            [{"tenant": i, "name": "Amazon"} for i in range(3)], exhaustive=True
        ).total_matches
        == 0
    )


def test_exact_baseline_survives_unknown_semantic_tokens() -> None:
    finder = Finder(
        schema=Schema(fields={"text": Text(mode="semantic")}), model_path=TINY, offline=True
    )
    finder.add([{"text": "hello world"}, {"text": "different unknown"}])
    union = finder.search({"text": "hello world"})
    oracle = finder.search({"text": "hello world"}, exhaustive=True)
    assert union.matches == oracle.matches
    assert len(union.matches) == 1
    match = union.matches[0]
    assert match.kind == "exact"
    assert match.score == match.coverage == match.field_scores["text"] == 1.0
    assert finder.find([{"text": "hello world"}] * 2).total_matches == 1
    assert finder.find([{"text": "hello world"}] * 2, exhaustive=True).total_matches == 1
