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
    _univo,
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


@pytest.mark.parametrize("minimum", [0.0, -1.0, 1.0000000001, float("nan"), float("inf"), 1e-300])
@pytest.mark.parametrize("offline", [False, True])
def test_invalid_threshold_never_resolves_model(
    monkeypatch: pytest.MonkeyPatch, minimum: float, offline: bool
) -> None:
    def forbidden_load(*args: object, **kwargs: object) -> None:
        pytest.fail("invalid options must fail before loading the model")

    monkeypatch.setattr("univo.models._load", forbidden_load)
    finder = Finder(schema=Schema(fields={"text": Text(mode="semantic")}), offline=offline)
    with pytest.raises(InvalidParameterError) as failure:
        finder.find([{"text": "online"}], min_score=minimum)
    assert failure.value.code == "parameter.threshold"
    with pytest.raises(InvalidParameterError) as failure:
        finder.search({"text": "online"}, min_score=minimum)
    assert failure.value.code == "parameter.threshold"
    assert finder.stats()["records"] == 0
    assert not finder.config()["model_loaded"]


def test_oversized_object_key_fails_before_child_path_allocation() -> None:
    finder = Finder(schema=Schema(fields={"v": Exact()}, max_selected_bytes=32))
    before = finder.stats()
    with pytest.raises(ResourceLimitError) as failure:
        finder.add([{"v": {"x" * (1 << 20): 1}}])
    assert failure.value.code == "limit.record_size"
    # Early refusal points to the containing object, without copying the huge key into a path.
    assert failure.value.path == "v"
    assert failure.value.record_index == 0
    assert finder.stats() == before


def test_semantic_candidate_find_requires_explicit_scan_budget() -> None:
    schema = Schema(fields={"text": Text(mode="semantic")})
    records = [{"text": "online"}, {"text": "cloud"}, {"text": "payments"}]
    finder = Finder(schema=schema, model_path=TINY, offline=True, max_semantic_comparisons=8)
    for limit in (0, 10):
        with pytest.raises(ResourceLimitError) as failure:
            finder.find(records, limit=limit, max_entries=0)
        assert failure.value.code == "limit.semantic_comparisons"
        assert failure.value.details["requested"] == 9
        assert finder.stats()["records"] == 0
    allowed = Finder(schema=schema, model_path=TINY, offline=True, max_semantic_comparisons=9)
    assert allowed.find(records).matches == allowed.find(records, exhaustive=True).matches
    assert allowed.config()["max_semantic_comparisons"] == 9


def test_semantic_scan_budget_respects_partitions_and_field_count() -> None:
    schema = Schema(
        partition_by=["tenant"],
        fields={
            "one": Text(mode="semantic"),
            "two": Text(mode="semantic"),
        },
    )
    records = [{"tenant": i // 2, "one": "online", "two": "cloud"} for i in range(3)]
    # Two fields, partitions of sizes 2 and 1: (2*2 + 1*1) * 2 = 10 entries.
    blocked = Finder(schema=schema, model_path=TINY, offline=True, max_semantic_comparisons=9)
    with pytest.raises(ResourceLimitError) as failure:
        blocked.find(records)
    assert failure.value.details["requested"] == 10
    allowed = Finder(schema=schema, model_path=TINY, offline=True, max_semantic_comparisons=10)
    assert allowed.find(records).total_matches == 1


def test_quality_profile_runner_preserves_every_critical_hard_gate() -> None:
    from quality_profiles import evaluate

    corpus = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "critical" / "cases.jsonl"
    for rows, pairs, failures in (
        evaluate(corpus, "lexical", TINY),
        evaluate(corpus, "semantic", TINY),
    ):
        assert failures == []
        assert len(rows) == len(pairs) == 178


def test_semantic_cache_is_shared_and_counts_normalized_texts() -> None:
    finder = Finder(
        schema=Schema(fields={"text": Text(mode="semantic")}), model_path=TINY, offline=True
    )
    assert finder.stats()["embedding_cache_bytes"] == 0
    finder.add([{"text": "ONLINE"}])
    assert finder.stats()["embedding_cache_misses"] == 1
    first = finder.search({"text": "online", "unused": "before"})
    second = finder.search({"text": "ONLINE", "unused": "after"})
    assert first.matches[0].score == second.matches[0].score
    assert first.matches[0].kind == second.matches[0].kind
    assert first.matches[0].right_id == second.matches[0].right_id
    assert first.matches[0].reasons != second.matches[0].reasons
    finder.find([{"text": "online"}, {"text": "ONLINE"}])
    stats = finder.stats()
    assert stats["embedding_cache_misses"] == 1
    assert stats["embedding_cache_hits"] == 3
    assert stats["embedding_cache_entries"] == 1
    maximum = finder.config()["embedding_cache_bytes"]
    assert isinstance(maximum, int)
    assert 0 < stats["embedding_cache_bytes"] <= maximum


def test_cache_reuses_only_unchanged_semantic_fields() -> None:
    finder = Finder(
        schema=Schema(fields={"one": Text(mode="semantic"), "two": Text(mode="semantic")}),
        model_path=TINY,
        offline=True,
    )
    finder.add([{"one": "online", "two": "cloud"}])
    finder.search({"one": "online", "two": "payments"})
    assert finder.stats()["embedding_cache_misses"] == 3
    assert finder.stats()["embedding_cache_hits"] == 1
    finder.search({"one": "online", "two": "payments"})
    assert finder.stats()["embedding_cache_misses"] == 3
    assert finder.stats()["embedding_cache_hits"] == 3


@pytest.mark.parametrize("budget", [0, 1, 512])
def test_cache_budget_preserves_matches_and_can_disable_retention(budget: int) -> None:
    schema = Schema(fields={"text": Text(mode="semantic")})
    cached = Finder(schema=schema, model_path=TINY, offline=True, embedding_cache_bytes=budget)
    plain = Finder(schema=schema, model_path=TINY, offline=True, embedding_cache_bytes=0)
    records = [{"text": "online"}, {"text": "cloud"}, {"text": "unknown"}]
    assert cached.find(records).matches == plain.find(records).matches
    assert (
        cached.find(records, exhaustive=True).matches
        == plain.find(records, exhaustive=True).matches
    )
    assert cached.stats()["embedding_cache_bytes"] <= budget
    if budget < 512:
        assert cached.stats()["embedding_cache_entries"] == 0


@pytest.mark.parametrize("budget", [-1, True, 0.5, 2**63])
def test_invalid_cache_budget_fails_at_construction(budget: int) -> None:
    with pytest.raises(InvalidParameterError):
        Finder(embedding_cache_bytes=budget)


def test_an_acronym_matches_the_words_it_abbreviates() -> None:
    finder = Finder(schema=Schema(id_field="id", fields={"name": Text()}))
    finder.add(
        [
            {"id": "long", "name": "National Institute for Occupational Safety and Health"},
            {"id": "other", "name": "National Health Service"},
        ]
    )
    report = finder.search({"id": "short", "name": "N.I.O.S.H."})
    assert [(m.right_id, m.reasons) for m in report.matches] == [("long", ["name: Acronym"])]
    assert report.matches[0].score == pytest.approx(0.9)
    # Three letters name too many organizations to count as evidence.
    assert finder.search({"id": "nhs", "name": "NHS"}).matches == ()


def test_names_with_distinct_words_score_lower_and_conflict() -> None:
    finder = Finder(schema=Schema(id_field="id", fields={"name": Text()}), offline=True)
    template = finder.find(
        [
            {"id": "a", "name": "National Institute of Epidemiology"},
            {"id": "b", "name": "National Institute of Oncology"},
        ],
        min_score=0.01,
    ).matches
    extended = finder.find(
        [
            {"id": "a", "name": "National Institute of Oncology"},
            {"id": "b", "name": "National Institute of Oncology Research"},
        ],
        min_score=0.01,
    ).matches
    assert template[0].conflicts == ["name"]
    assert extended[0].conflicts == []
    assert template[0].score < extended[0].score


def test_the_reported_versions_are_the_native_ones() -> None:
    config = Finder().config()
    assert (config["scoring_version"], config["search_version"], config["normalizer_version"]) == (
        _univo.SCORING_VERSION,
        _univo.SEARCH_VERSION,
        _univo.NORMALIZER_VERSION,
    )
    assert _univo.SCORING_VERSION == 4
