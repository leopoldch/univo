"""Mutation contracts tested against an installed native wheel."""

import hashlib
import json
import pathlib
from collections.abc import Callable, Iterator
from typing import Any
from unittest.mock import patch

import pytest
from univo import (
    DuplicateIdError,
    Exact,
    Finder,
    IndexCompatibilityError,
    InvalidParameterError,
    InvalidRecordError,
    ResourceLimitError,
    Schema,
    Text,
)

TINY = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "models" / "tiny"


def finder(capacity: int = 4) -> Finder:
    return Finder(
        schema=Schema(
            id_field="id",
            partition_by=["tenant"],
            fields={"name": Text(), "env": Exact(conflict="reject")},
        ),
        max_records=capacity,
        offline=True,
    )


def row(identifier: int | str, name: str = "Amazon", tenant: str = "a") -> dict[str, object]:
    return {"id": identifier, "name": name, "tenant": tenant, "env": "prod"}


def test_typed_ids_replacements_partitions_removal_and_capacity() -> None:
    index = finder(2)
    assert index.add([row(1), row("1")]) == [1, "1"]
    assert index.upsert([row(1, "Microsoft", "b")]) == [1]
    assert index.search(row("query")).matches[0].right_id == "1"
    assert index.search(row("query", "Microsoft", "b")).matches[0].right_id == 1
    assert index.stats()["records"] == 2
    assert index.stats()["generation"] == 2
    assert index.remove([1, 1]) == []
    assert index.remove(["missing", "missing"]) == ["missing", "missing"]
    assert index.stats()["generation"] == 3
    assert not index.search(row("query", "Microsoft", "b")).matches
    assert index.add([row(1)]) == [1]
    assert index.remove(["1"]) == []
    assert index.stats()["records"] == 1
    assert index.stats()["generation"] == 5


def test_missing_and_empty_batches_leave_generation_unchanged() -> None:
    index = finder()
    with patch("univo.models._load", side_effect=AssertionError("model loaded")):
        assert index.remove([1, "1"]) == [1, "1"]
        assert index.stats()["generation"] == 0
    index.add([row(1)])
    assert index.upsert([]) == []
    assert index.remove([]) == []
    assert index.remove(["unknown"]) == ["unknown"]
    assert index.stats()["generation"] == 1
    assert index.upsert([row(1)]) == [1]
    assert index.stats()["generation"] == 2


def test_invalid_batch_or_generator_never_changes_active_state() -> None:
    index = finder(2)
    index.add([row(1)])
    before = index.search(row("query"))
    cases: list[tuple[list[object], type[Exception]]] = [
        ([row(1, "Microsoft"), row(1)], DuplicateIdError),
        ([row(1, "Microsoft"), {"id": 2}], InvalidRecordError),
        ([row(1, "Microsoft"), row(2), row(3)], ResourceLimitError),
    ]
    for records, error in cases:
        with pytest.raises(error):
            index.upsert(records)
        assert index.search(row("query")) == before
    with pytest.raises(InvalidRecordError):
        index.remove([1, True])
    assert index.search(row("query")) == before

    def broken() -> Iterator[object]:
        yield row(1, "Microsoft")
        raise RuntimeError("iterator failure")

    with pytest.raises(RuntimeError, match="iterator failure"):
        index.upsert(broken())
    assert index.search(row("query")) == before


@pytest.mark.parametrize("identifier", [None, False, 1.0, [], {}, 1 << 64])
def test_invalid_remove_ids_are_checked_even_before_first_add(identifier: Any) -> None:
    index = finder()
    with pytest.raises(InvalidRecordError):
        index.remove([identifier])
    assert index.stats()["generation"] == 0


def test_mutations_require_id_field_before_model_resolution() -> None:
    index = Finder(schema=Schema(fields={"name": Text(mode="semantic")}), offline=True)
    with patch("univo.models._load", side_effect=AssertionError("model loaded")):
        with pytest.raises(InvalidParameterError):
            index.upsert([])
        with pytest.raises(InvalidParameterError):
            index.remove([])


def test_semantic_mutation_save_load_reuses_unchanged_fields_without_cache(
    tmp_path: pathlib.Path,
) -> None:
    index = Finder(
        schema=Schema(id_field="id", fields={"name": Text(), "description": Text(mode="semantic")}),
        model_path=TINY,
        offline=True,
        embedding_cache_bytes=0,
        max_records=4,
    )
    index.add(
        [
            {"id": 1, "name": "DELETED-SECRET", "description": "world"},
            {"id": 2, "name": "old", "description": "hello"},
        ]
    )
    index.remove([1])
    index.upsert([{"id": 2, "name": "new", "description": "hello"}])
    path = tmp_path / "index"
    index.save(path)
    with patch("urllib.request.urlopen", side_effect=AssertionError("network")):
        loaded = Finder.load(path, offline=True)
    assert loaded.stats()["generation"] == 3
    assert loaded.stats()["records"] == 1
    assert loaded.upsert([{"id": 2, "name": "latest", "description": "HELLO"}]) == [2]
    query = {"id": "query", "name": "latest", "description": "hello"}
    assert loaded.search(query).matches[0].right_id == 2
    assert loaded.stats()["embedding_cache_misses"] == 0
    for file in path.rglob("*"):
        if file.is_file():
            assert b"deleted-secret" not in file.read_bytes()
    loaded.remove([2])
    loaded.save(path)
    assert Finder.load(path, offline=True).stats()["records"] == 0


def test_generation_overflow_rejects_mutation_before_publication(tmp_path: pathlib.Path) -> None:
    index = finder()
    index.add([row(1)])
    path = tmp_path / "index"
    index.save(path)
    generation = path / (path / "CURRENT").read_text().splitlines()[0]
    manifest = generation / "manifest.json"
    content = json.loads(manifest.read_text())
    content["generation"] = (1 << 64) - 1
    encoded = json.dumps(content).encode()
    manifest.write_bytes(encoded)
    checksum = hashlib.sha256(encoded).hexdigest()
    (generation / "COMPLETE").write_text(checksum)
    (path / "CURRENT").write_text(f"{generation.name}\n{checksum}\n")
    loaded = Finder.load(path, offline=True)
    before = loaded.search(row("query"))
    operations: list[Callable[[], object]] = [
        lambda: loaded.upsert([row(1, "Microsoft")]),
        lambda: loaded.remove([1]),
    ]
    for operation in operations:
        with pytest.raises(IndexCompatibilityError):
            operation()
        assert loaded.search(row("query")) == before
    assert loaded.remove(["missing"]) == ["missing"]


def test_readers_observe_one_complete_generation_during_upserts() -> None:
    from concurrent.futures import ThreadPoolExecutor

    index = finder()
    index.add([row(1)])

    def write() -> None:
        for step in range(100):
            index.upsert([row(1, "Microsoft" if step % 2 == 0 else "Amazon")])

    def read() -> None:
        for _ in range(100):
            report = index.search(row("query"), min_score=1.0)
            assert report.total_matches == report.generation % 2

    with ThreadPoolExecutor(max_workers=2) as pool:
        writer = pool.submit(write)
        reader = pool.submit(read)
        writer.result()
        reader.result()
    assert index.stats()["generation"] == 101
