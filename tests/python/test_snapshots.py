"""Installed-wheel snapshot contracts, using synthetic records and models."""

import json
import pathlib
from unittest.mock import patch

import pytest
from univo import Exact, Finder, IndexCompatibilityError, ResourceLimitError, Schema, Text

TINY = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "models" / "tiny"


def test_round_trip_selected_typed_values_partitions_and_explanations(
    tmp_path: pathlib.Path,
) -> None:
    schema = Schema(
        id_field="id",
        partition_by=["tenant"],
        fields={
            "nested.name": Text(),
            "data": Exact(),
            "env": Exact(conflict="reject"),
        },
    )
    records = [
        {
            "id": 1,
            "tenant": "x",
            "nested": {"name": "CAFE\u0301"},
            "data": {"b": [0, False, 1.0, None], "a": "e\u0301"},
            "env": "prod",
            "secret": "UNSELECTED-SECRET",
        },
        {
            "id": "1",
            "tenant": "x",
            "nested": {"name": "café"},
            "data": {"a": "é", "b": [0, False, 1.0, None]},
            "env": "prod",
        },
        {"id": 3, "tenant": "y", "nested": {"name": "café"}, "data": None},
        {"id": 4, "tenant": "x", "nested": {"name": "café"}, "env": "test"},
    ]
    finder = Finder(schema=schema, offline=True)
    finder.add(records[:2])
    finder.add(records[2:])
    before = finder.search(records[0], min_score=0.5, exhaustive=True)
    finder.save(tmp_path / "index")
    with patch("urllib.request.urlopen", side_effect=AssertionError("network")):
        loaded = Finder.load(tmp_path / "index", offline=True)
    assert loaded.search(records[0], min_score=0.5, exhaustive=True) == before
    assert loaded.stats()["generation"] == 2
    assert loaded.add([{"id": 5, "tenant": "x", "nested": {"name": "other"}}]) == [5]
    assert loaded.stats()["generation"] == 3
    for file in (tmp_path / "index").rglob("*"):
        if file.is_file():
            assert b"UNSELECTED-SECRET" not in file.read_bytes()


def test_semantic_roundtrip_offline_keeps_vectors_and_resets_cache(tmp_path: pathlib.Path) -> None:
    finder = Finder(
        schema=Schema(fields={"name": Text(mode="semantic")}), model_path=TINY, offline=True
    )
    finder.add([{"name": "hello"}, {"name": "world"}, {"name": "UNKNOWN"}, {}])
    before = finder.search({"name": "hello"}, min_score=0.1, exhaustive=True)
    finder.save(tmp_path / "index")
    with (
        patch(
            "univo.models._load", side_effect=AssertionError("model resolved before capacity check")
        ),
        pytest.raises(ResourceLimitError),
    ):
        Finder.load(tmp_path / "index", offline=True, max_records=1)
    with patch("urllib.request.urlopen", side_effect=AssertionError("network")):
        loaded = Finder.load(tmp_path / "index", offline=True)
    assert loaded.stats()["embedding_cache_entries"] == 0
    assert loaded.search({"name": "hello"}, min_score=0.1, exhaustive=True) == before
    assert loaded.config()["model_loaded"]


def test_strings_empty_and_repeated_saves(tmp_path: pathlib.Path) -> None:
    finder = Finder(offline=True)
    path = tmp_path / "index"
    finder.save(path)
    loaded = Finder.load(path, offline=True)
    assert loaded.stats()["records"] == 0
    assert not loaded.config()["model_loaded"]
    loaded.add(["Amazon", "AMAZON"])
    loaded.save(path)
    loaded.add(["Microsoft"])
    assert Finder.load(path, offline=True).stats()["records"] == 2
    loaded.save(path)
    assert Finder.load(path, offline=True).stats()["records"] == 3


def test_failed_save_and_load_limits(tmp_path: pathlib.Path) -> None:
    finder = Finder(offline=True)
    finder.add(["old"])
    path = tmp_path / "index"
    finder.save(path)
    finder.add(["x" * 1000])
    with pytest.raises(ResourceLimitError):
        finder.save(path, max_snapshot_bytes=100)
    assert Finder.load(path, offline=True).stats()["records"] == 1
    with pytest.raises(ResourceLimitError):
        Finder.load(path, offline=True, max_snapshot_bytes=100)
    finder.save(path)
    with pytest.raises(ResourceLimitError):
        Finder.load(path, offline=True, max_records=1)


@pytest.mark.parametrize("name", ["records.bin", "vectors.bin", "manifest.json", "COMPLETE"])
def test_corrupt_or_incomplete_snapshots_fail(tmp_path: pathlib.Path, name: str) -> None:
    finder = Finder(offline=True)
    finder.add(["Amazon"])
    path = tmp_path / "index"
    finder.save(path)
    generation = (path / "CURRENT").read_text().splitlines()[0]
    file = path / generation / name
    data = bytearray(file.read_bytes())
    data[-1] ^= 1
    file.write_bytes(data)
    with pytest.raises(IndexCompatibilityError):
        Finder.load(path, offline=True)


def test_saved_model_is_pinned_and_changed_local_files_rejected(tmp_path: pathlib.Path) -> None:
    import shutil

    model = tmp_path / "model"
    shutil.copytree(TINY, model)
    finder = Finder(schema=Schema(fields={"name": Text(mode="semantic")}), model_path=model)
    finder.add([{"name": "hello"}])
    path = tmp_path / "index"
    finder.save(path)
    generation = (path / "CURRENT").read_text().splitlines()[0]
    manifest = json.loads((path / generation / "manifest.json").read_text())
    assert manifest["dimension"] > 0
    assert manifest["model"]["files"]
    assert manifest["configuration"]["resolved_model"] == str(model)
    config = model / "config.json"
    config.write_bytes(config.read_bytes() + b"\n")
    with pytest.raises(IndexCompatibilityError, match="model identity"):
        Finder.load(path, offline=True)


@pytest.mark.skipif(__import__("os").name != "posix", reason="POSIX permission bits")
def test_snapshot_permissions_are_private(tmp_path: pathlib.Path) -> None:
    Finder(offline=True).save(tmp_path / "index")
    for path in (tmp_path / "index").rglob("*"):
        assert path.stat().st_mode & 0o077 == 0


def test_hub_snapshot_load_uses_saved_commit_and_never_http_offline(tmp_path: pathlib.Path) -> None:
    from univo.models import _load

    from ._hub import COMMIT, REPOSITORY, Hub, serve

    cache = tmp_path / "cache"
    with serve(Hub()) as hub:
        finder = Finder(
            schema=Schema(fields={"name": Text(mode="semantic")}),
            model=REPOSITORY,
            cache_dir=cache,
            hub_url=hub.url,
        )
        finder.add([{"name": "hello"}])
        finder.save(tmp_path / "index")
    with (
        patch("urllib.request.urlopen", side_effect=AssertionError("network")),
        patch("univo.models._load", wraps=_load) as load,
    ):
        loaded = Finder.load(tmp_path / "index", offline=True, cache_dir=cache)
        assert load.call_args.args[0] == f"{REPOSITORY}@{COMMIT}"
        assert loaded.search({"name": "hello"}).matches
        from univo import ModelUnavailableError

        with pytest.raises(ModelUnavailableError):
            Finder.load(tmp_path / "index", offline=True, cache_dir=tmp_path / "absent")


@pytest.mark.skipif(__import__("os").name != "posix", reason="POSIX permissions")
def test_read_only_destination_leaves_current_snapshot(tmp_path: pathlib.Path) -> None:
    import os

    if os.geteuid() == 0:
        pytest.skip("root bypasses permission checks")
    path = tmp_path / "index"
    finder = Finder(offline=True)
    finder.add(["old"])
    finder.save(path)
    finder.add(["new"])
    path.chmod(0o500)
    try:
        with pytest.raises(IndexCompatibilityError) as failure:
            finder.save(path)
        assert failure.value.code == "state.snapshot_io"
        assert Finder.load(path, offline=True).stats()["records"] == 1
    finally:
        path.chmod(0o700)


def test_giant_metadata_is_rejected_before_json_copy_or_model_load(tmp_path: pathlib.Path) -> None:
    from univo import InvalidParameterError

    finder = Finder(schema=Schema(fields={"x" * (2 << 20): Text(mode="semantic")}), offline=True)
    with (
        patch("json.dumps", side_effect=AssertionError("copied giant metadata")),
        patch("univo.models._load", side_effect=AssertionError("resolved model")),
        pytest.raises(InvalidParameterError),
    ):
        finder.save(tmp_path / "index")
    assert not (tmp_path / "index").exists()
