"""check_or_add and concurrent use of one Finder from several threads."""

import threading
from concurrent.futures import ThreadPoolExecutor

import pytest
from univo import DuplicateIdError, Finder, InvalidParameterError, Schema, Text

NAMES = (
    "Acme Corporation",
    "Globex Industries",
    "Initech Software",
    "Umbrella Biotech",
    "Stark Engineering",
    "Wayne Logistics",
    "Tyrell Robotics",
    "Cyberdyne Systems",
)


def finder() -> Finder:
    return Finder(schema=Schema(fields={"name": Text()}, id_field="id"))


def test_check_or_add_inserts_new_records_and_reports_others() -> None:
    index = finder()
    first = index.check_or_add({"id": 1, "name": "Acme Corporation"})
    assert (first.status, first.id) == ("inserted", 1)
    assert first.report.matches == ()
    assert index.stats()["generation"] == 1

    equal = index.check_or_add({"id": 2, "name": "ACME corporation"})
    assert (equal.status, equal.id) == ("matched", None)
    assert [(found.right_id, found.kind) for found in equal.report.matches] == [(1, "exact")]

    similar = index.check_or_add({"id": 3, "name": "Acme Corporatoin"})
    assert (similar.status, similar.id) == ("review", None)
    assert similar.report.matches[0].kind == "candidate"

    assert index.stats()["records"] == 1
    assert index.stats()["generation"] == 1


def test_check_or_add_without_ids_returns_the_new_slot() -> None:
    index = Finder()
    assert index.check_or_add("Acme").id == 0
    assert index.check_or_add("Globex").id == 1
    assert index.check_or_add("acme").status == "matched"


def test_failed_check_or_add_inserts_nothing() -> None:
    index = finder()
    index.add([{"id": 1, "name": "Acme Corporation"}])
    with pytest.raises(DuplicateIdError):
        index.check_or_add({"id": 1, "name": "Globex"})
    with pytest.raises(InvalidParameterError):
        index.check_or_add({"id": 2, "name": "Globex"}, min_score=0)
    assert index.stats()["records"] == 1
    assert index.stats()["generation"] == 1


def test_racing_threads_insert_each_duplicate_once() -> None:
    index = finder()
    threads = 8
    for round_, base in enumerate(NAMES):
        barrier = threading.Barrier(threads)

        def attempt(
            thread: int,
            base: str = base,
            round_: int = round_,
            barrier: threading.Barrier = barrier,
        ) -> str:
            # The same name, with a typo in odd threads.
            name = base if thread % 2 == 0 else base + "e"
            barrier.wait()
            decision = index.check_or_add({"id": round_ * 100 + thread, "name": name})
            return decision.status

        with ThreadPoolExecutor(threads) as pool:
            statuses = list(pool.map(attempt, range(threads)))
        assert statuses.count("inserted") == 1, statuses
    assert index.stats()["records"] == len(NAMES)


def test_searches_run_alongside_changes() -> None:
    index = finder()
    index.add([{"id": i, "name": f"{NAMES[i % len(NAMES)]} {i}"} for i in range(200)])
    stop = threading.Event()
    seen: list[list[int]] = []
    errors: list[BaseException] = []

    def search() -> None:
        generations = []
        try:
            while not stop.is_set():
                report = index.search({"id": -1, "name": "Acme Corporation 8"}, min_score=0.5)
                assert report.total_matches >= 1
                generations.append(report.generation)
        except BaseException as error:  # reported by the main thread
            errors.append(error)
        seen.append(generations)

    readers = [threading.Thread(target=search) for _ in range(4)]
    for reader in readers:
        reader.start()
    for i in range(200, 260):
        index.add([{"id": i, "name": f"Other {i}"}])
        index.remove([i - 1]) if i % 2 else index.upsert([{"id": i, "name": f"Renamed {i}"}])
    stop.set()
    for reader in readers:
        reader.join()

    assert errors == []
    assert all(generations == sorted(generations) for generations in seen)
    assert any(generations for generations in seen)
    stats = index.stats()
    assert stats["records"] == 230
    assert stats["generation"] == 1 + 60 * 2


def test_the_documented_example_runs() -> None:
    import pathlib

    document = pathlib.Path(__file__).resolve().parents[2] / "docs" / "concurrency.md"
    text = document.read_text(encoding="utf-8")
    example = text.split("```python\n", 1)[1].split("```", 1)[0]
    exec(compile(example, str(document), "exec"), {})
