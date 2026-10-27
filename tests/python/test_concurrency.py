"""check_or_add and concurrent use of one Finder from several threads."""

import pathlib
import random
import threading
from concurrent.futures import ThreadPoolExecutor
from typing import Literal

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

    document = pathlib.Path(__file__).resolve().parents[2] / "docs" / "design" / "concurrency.md"
    text = document.read_text(encoding="utf-8")
    example = text.split("```python\n", 1)[1].split("```", 1)[0]
    exec(compile(example, str(document), "exec"), {})


@pytest.mark.parametrize("mode", ["lexical", "semantic"])
def test_find_and_group_give_the_same_result_on_every_thread_budget(
    mode: Literal["lexical", "semantic"],
) -> None:
    tiny = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "models" / "tiny"
    rng = random.Random(3)
    records = []
    for row in range(600):
        name = list(rng.choice(NAMES))
        for _ in range(rng.randrange(3)):
            name[rng.randrange(len(name))] = rng.choice("aex")
        records.append({"id": row, "name": "".join(name)})

    def run(threads: int) -> tuple[list[tuple[str | int, str | int, float]], int, object]:
        index = Finder(
            schema=Schema(fields={"name": Text(mode=mode)}, id_field="id"),
            model_path=tiny if mode == "semantic" else None,
            offline=True,
            threads=threads,
        )
        assert index.config()["threads"] == threads
        report = index.find(records, min_score=0.6, limit=500)
        pairs = [(m.left_id, m.right_id, m.score) for m in report.matches]
        groups = index.group(records, min_score=0.6)
        return pairs, report.total_matches, (groups.exact, groups.groups)

    expected = run(1)
    assert expected[1] > 500
    assert run(4) == expected


def test_threads_default_to_every_available_core() -> None:
    import univo._univo as native

    available = native.available_threads()
    assert available >= 1
    assert finder().config()["threads"] == available
    assert Finder(fields=["name"], threads=None).config()["threads"] == available
    for invalid in (0, -1, 1.5, "2", True):
        with pytest.raises(InvalidParameterError):
            Finder(fields=["name"], threads=invalid)  # type: ignore[arg-type]


def test_a_finder_creates_its_worker_pool_once_and_starts_it_on_need() -> None:
    index = Finder(schema=Schema(fields={"name": Text()}, id_field="id"), threads=2)
    small = [{"id": row, "name": name} for row, name in enumerate(NAMES)]
    index.find(small)
    pool = index._pool
    assert pool is not None
    assert pool.threads == 2
    # A small report runs on the calling thread: no worker starts.
    assert not pool.started
    large = [{"id": row, "name": f"{NAMES[row % len(NAMES)]} {row}"} for row in range(600)]
    index.find(large)
    assert pool.started
    index.group(large)
    index.add(small)
    assert index._pool is pool
    # One thread without a model needs no pool: work runs on the caller.
    single = Finder(schema=Schema(fields={"name": Text()}, id_field="id"), threads=1)
    single.find(large)
    assert single._pool is None


def test_lexical_add_and_search_start_no_worker() -> None:
    index = Finder(fields=["name"], threads=4)
    index.add([{"name": "Amazon"}, {"name": "Amazon Web Services"}])
    index.search({"name": "Amazon"})
    index.check_or_add({"name": "Amazn"})
    assert index._pool is None or not index._pool.started
