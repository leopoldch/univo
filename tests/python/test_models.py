"""Model download, cache and offline use, against a local stand-in hub."""

import array
import json
import math
import os
import pathlib
import socket
import subprocess
import sys
from collections.abc import Iterator

import pytest
import univo
from univo import _transport, _univo
from univo.errors import ModelIntegrityError, ModelUnavailableError, ResourceLimitError, UnivoError
from univo.models import download, inspect

from ._hub import COMMIT, REPOSITORY, TINY_MODEL, Hub, serve

MODEL = REPOSITORY


@pytest.fixture
def hub() -> Iterator[Hub]:
    with serve(Hub()) as running:
        yield running


@pytest.fixture(autouse=True)
def no_backoff(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("univo._transport.time.sleep", lambda seconds: None)


def file_requests(hub: Hub) -> list[str]:
    return [path for path in hub.requests if not path.startswith("/api/")]


def test_a_hub_model_is_downloaded_verified_and_then_served_from_the_cache(
    hub: Hub, tmp_path: pathlib.Path
) -> None:
    info = download(MODEL, hub_url=hub.url, cache_dir=tmp_path)

    assert info.name == f"{REPOSITORY}@{COMMIT}"
    assert (info.source, info.repository, info.revision) == ("hub", REPOSITORY, COMMIT)
    assert info.dimension == 8
    assert info.path == tmp_path / "models" / REPOSITORY / COMMIT
    assert [file.name for file in info.files] == [
        "config.json",
        "model.safetensors",
        "tokenizer.json",
    ]
    assert hub.requests == [
        f"/api/models/{REPOSITORY}/revision/main?blobs=true",
        f"/{REPOSITORY}/resolve/{COMMIT}/config.json",
        f"/{REPOSITORY}/resolve/{COMMIT}/model.safetensors",
        "/cdn/model.safetensors",
        f"/{REPOSITORY}/resolve/{COMMIT}/tokenizer.json",
    ]
    assert hub.user_agents == {f"univo/{univo.__version__}"}

    requests = len(hub.requests)
    assert download(MODEL, hub_url=hub.url, cache_dir=tmp_path) == info
    assert inspect(MODEL, cache_dir=tmp_path) == info
    assert inspect(f"{REPOSITORY}@{COMMIT}", cache_dir=tmp_path) == info
    assert len(hub.requests) == requests


def test_hf_endpoint_names_the_mirror(
    hub: Hub, tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("HF_ENDPOINT", hub.url)
    assert download(MODEL, cache_dir=tmp_path).revision == COMMIT


def test_transient_failures_are_retried_a_bounded_number_of_times(
    hub: Hub, tmp_path: pathlib.Path
) -> None:
    hub.statuses = {"config.json": [503, 429]}
    download(MODEL, hub_url=hub.url, cache_dir=tmp_path)
    assert file_requests(hub).count(f"/{REPOSITORY}/resolve/{COMMIT}/config.json") == 3

    hub.statuses = {"tokenizer.json": [500, 500, 500, 500]}
    with pytest.raises(ModelUnavailableError) as raised:
        download(f"{MODEL}@{COMMIT[:8]}", hub_url=hub.url, cache_dir=tmp_path / "other")
    assert raised.value.code == "model.download_failed"
    assert raised.value.details["file"] == "tokenizer.json"
    assert "HTTP 500" in str(raised.value)
    assert hub.statuses["tokenizer.json"] == [500]


def test_a_silent_server_times_out_after_a_bounded_number_of_attempts(
    tmp_path: pathlib.Path,
) -> None:
    with socket.create_server(("127.0.0.1", 0)) as silent:
        port = silent.getsockname()[1]
        transport = _transport.UrllibTransport(timeout=0.2, attempts=2)
        with pytest.raises(_transport.TransportError) as raised:
            transport.fetch(f"http://127.0.0.1:{port}/file", tmp_path / "file", 10)
    assert raised.value.reason == "no response for 0.2 s"
    assert raised.value.status is None


def test_a_missing_repository_is_reported_without_retrying(
    hub: Hub, tmp_path: pathlib.Path
) -> None:
    with pytest.raises(ModelUnavailableError) as raised:
        download("univo-test/absent", hub_url=hub.url, cache_dir=tmp_path)
    assert raised.value.code == "model.unavailable"
    assert "HTTP 404" in str(raised.value)
    assert len(hub.requests) == 1


def test_altered_files_are_rejected_and_nothing_is_cached(hub: Hub, tmp_path: pathlib.Path) -> None:
    hub.corrupt = {"model.safetensors"}
    with pytest.raises(ModelIntegrityError) as raised:
        download(MODEL, hub_url=hub.url, cache_dir=tmp_path)
    assert raised.value.code == "model.integrity"
    assert raised.value.details["file"] == "model.safetensors"
    with pytest.raises(ModelUnavailableError):
        inspect(MODEL, cache_dir=tmp_path)


def test_responses_larger_than_announced_are_cut_off(hub: Hub, tmp_path: pathlib.Path) -> None:
    hub.oversized = {"tokenizer.json"}
    with pytest.raises(ModelUnavailableError) as raised:
        download(MODEL, hub_url=hub.url, cache_dir=tmp_path)
    assert raised.value.code == "model.download_failed"
    assert "exceeds" in str(raised.value)
    assert file_requests(hub).count(f"/{REPOSITORY}/resolve/{COMMIT}/tokenizer.json") == 1


def test_offline_inspection_of_a_missing_model_names_the_cache(tmp_path: pathlib.Path) -> None:
    with pytest.raises(ModelUnavailableError) as raised:
        inspect("default", cache_dir=tmp_path)
    assert raised.value.code == "model.unavailable"
    assert str(tmp_path) in str(raised.value)
    assert "python -m univo.models download" in raised.value.hint


def test_local_directories_are_loaded_in_place() -> None:
    info = inspect(str(TINY_MODEL))
    assert (info.source, info.repository, info.revision) == ("local", None, None)
    assert info.path == TINY_MODEL
    assert all(file.digest.startswith("sha256:") for file in info.files)

    relative = inspect(f"./{os.path.relpath(TINY_MODEL)}").path
    assert relative.is_absolute()
    assert os.path.samefile(relative, TINY_MODEL)


def test_encoding_returns_unit_float32_rows_and_marks_texts_without_known_tokens() -> None:
    model = _univo.load_model(str(TINY_MODEL), transport=_transport.UrllibTransport())
    raw, absent = model.encode(["Amazon", "zzz", "amazon online retail"])

    vectors = array.array("f")
    vectors.frombytes(raw)
    assert len(vectors) == 3 * model.dimension
    rows = [vectors[index * 8 : (index + 1) * 8] for index in range(3)]
    assert absent == [1]
    assert all(value == 0.0 for value in rows[1])
    for row in (rows[0], rows[2]):
        assert math.isclose(math.fsum(value * value for value in row), 1.0, rel_tol=1e-5)

    with pytest.raises(TypeError):
        model.encode("amazon")  # type: ignore[arg-type]


def test_invalid_references_raise_typed_errors_with_every_part() -> None:
    with pytest.raises(ModelUnavailableError) as raised:
        inspect("potion")
    error = raised.value
    assert error.code == "model.invalid_spec"
    assert error.details == {"model": "potion"}
    assert error.hint.startswith('Pass "default"')
    assert error.documentation_url.endswith("#modelinvalid_spec")
    assert str(error).splitlines()[1] == f"hint: {error.hint}"


def test_every_error_kind_has_an_exception_class() -> None:
    for kind in _univo.ERROR_KINDS:
        assert issubclass(getattr(univo.errors, f"{kind}Error"), UnivoError)
        assert getattr(univo, f"{kind}Error") is getattr(univo.errors, f"{kind}Error")
    assert issubclass(ResourceLimitError, UnivoError)


class Interrupting:
    def fetch(self, url: str, destination: os.PathLike[str], max_bytes: int) -> None:
        raise KeyboardInterrupt

    def fetch_text(self, url: str, max_bytes: int) -> str:
        raise KeyboardInterrupt

    def keep_waiting(self) -> bool:
        return True


class Failing(Interrupting):
    def fetch_text(self, url: str, max_bytes: int) -> str:
        raise ValueError("no route")


def test_an_interruption_during_a_download_propagates_unchanged(tmp_path: pathlib.Path) -> None:
    with pytest.raises(KeyboardInterrupt):
        _univo.load_model(MODEL, transport=Interrupting(), cache_dir=str(tmp_path))


def test_other_transport_exceptions_become_download_failures(tmp_path: pathlib.Path) -> None:
    with pytest.raises(ModelUnavailableError) as raised:
        _univo.load_model(MODEL, transport=Failing(), cache_dir=str(tmp_path))
    assert raised.value.code == "model.download_failed"
    assert "no route" in str(raised.value)


NETWORK_FORBIDDEN = """
import json, sys

events = []

def forbid(event, args):
    if event.startswith("socket."):
        events.append(event)
        raise RuntimeError("network access is forbidden in this test")

sys.addaudithook(forbid)
from univo.errors import UnivoError
from univo.models import download, inspect

outcome = {{}}
try:
    outcome["inspect"] = inspect({model!r}, cache_dir={cache!r}).name
except UnivoError as error:
    outcome["inspect"] = error.code
outcome["offline_events"] = list(events)
try:
    outcome["download"] = download({model!r}, hub_url="http://192.0.2.1", cache_dir={cache!r}).name
except UnivoError as error:
    outcome["download"] = error.code
outcome["events"] = events
outcome["http_loaded"] = "urllib.request" in sys.modules or "http.client" in sys.modules
print(json.dumps(outcome))
"""


def run_without_network(model: str, cache: pathlib.Path) -> dict[str, object]:
    completed = subprocess.run(
        [sys.executable, "-I", "-c", NETWORK_FORBIDDEN.format(model=model, cache=str(cache))],
        capture_output=True,
        check=True,
        text=True,
    )
    result: dict[str, object] = json.loads(completed.stdout)
    return result


def test_a_cached_model_is_used_with_the_network_forbidden(
    hub: Hub, tmp_path: pathlib.Path
) -> None:
    info = download(MODEL, hub_url=hub.url, cache_dir=tmp_path)
    assert run_without_network(MODEL, tmp_path) == {
        "inspect": info.name,
        "offline_events": [],
        "download": info.name,
        "events": [],
        "http_loaded": False,
    }


def test_a_missing_model_fails_offline_without_any_network_attempt(
    tmp_path: pathlib.Path,
) -> None:
    result = run_without_network(MODEL, tmp_path)
    assert result["inspect"] == "model.unavailable"
    assert result["offline_events"] == []
    # Online, the download is attempted and the audit hook blocks it.
    assert result["download"] == "model.download_failed"
    assert result["events"] != []


def run_cli(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-I", "-m", "univo.models", *arguments],
        capture_output=True,
        text=True,
        check=False,
    )


def test_the_command_line_downloads_and_inspects(hub: Hub, tmp_path: pathlib.Path) -> None:
    cache = str(tmp_path)
    downloaded = run_cli("download", MODEL, "--hub-url", hub.url, "--cache-dir", cache, "--json")
    assert downloaded.returncode == 0, downloaded.stderr
    fields = json.loads(downloaded.stdout)
    assert (fields["revision"], fields["dimension"]) == (COMMIT, 8)

    inspected = run_cli("inspect", MODEL, "--cache-dir", cache)
    assert inspected.returncode == 0, inspected.stderr
    assert f"model      {REPOSITORY}@{COMMIT}" in inspected.stdout

    missing = run_cli("inspect", "default", "--cache-dir", cache)
    assert missing.returncode == 1
    assert "code: model.unavailable" in missing.stderr
    assert missing.stdout == ""


@pytest.mark.skipif(
    "UNIVO_TEST_MODEL_DIR" not in os.environ, reason="UNIVO_TEST_MODEL_DIR is not set"
)
def test_the_default_model_downloads_from_a_mirror_and_loads(tmp_path: pathlib.Path) -> None:
    mirror = Hub(
        repository="minishlab/potion-base-8M",
        commit="bf8b056651a2c21b8d2565580b8569da283cab23",
        directory=pathlib.Path(os.environ["UNIVO_TEST_MODEL_DIR"]),
    )
    with serve(mirror):
        info = download("default", hub_url=mirror.url, cache_dir=tmp_path)
    assert (info.revision, info.dimension) == (mirror.commit, 256)
    assert not any(path.startswith("/api/") for path in mirror.requests)
    assert inspect("default", cache_dir=tmp_path) == info
