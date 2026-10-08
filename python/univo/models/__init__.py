"""Embedding models: download, inspection and offline use.

A model is named by a string: ``"default"``, a Hugging Face repository
``"org/name"`` or ``"org/name@revision"``, or a local directory starting
with ``/``, ``./``, ``../``, ``~`` or ``file://``. Hub models are kept in a
verified cache under ``UNIVO_CACHE_DIR`` or the platform's cache directory,
and are served from it without any request.

From a shell, ``python -m univo.models download default`` prepares an
image or a machine for offline use.
"""

from __future__ import annotations

import os
from typing import TYPE_CHECKING, Literal, NamedTuple, cast

from univo import _univo

if TYPE_CHECKING:
    import pathlib

__all__ = ["ModelFile", "ModelInfo", "download", "inspect"]


class ModelFile(NamedTuple):
    name: str
    size: int
    """Size in bytes."""
    digest: str
    """``sha256:<hex>``, or ``git-blob-sha1:<hex>`` for small hub files."""


class ModelInfo(NamedTuple):
    name: str
    """``org/name@commit`` for a hub model, the directory for a local one."""
    source: Literal["hub", "local"]
    repository: str | None
    revision: str | None
    """Full commit hash of a hub model."""
    path: pathlib.Path
    dimension: int
    backend: str
    files: tuple[ModelFile, ...]


def download(
    model: str = "default",
    *,
    hub_url: str | None = None,
    cache_dir: str | os.PathLike[str] | None = None,
) -> ModelInfo:
    """Downloads ``model`` into the cache unless it is already there, verifies
    and loads it.

    ``hub_url`` names a Hugging Face compatible mirror; it defaults to
    ``HF_ENDPOINT``, then to https://huggingface.co. Only repository and file
    names are sent.

    Raises:
        ModelUnavailableError: The model cannot be found or downloaded.
        ModelIntegrityError: A file fails verification.
        ResourceLimitError: The model exceeds the download size limit.
    """
    return _info(_load(model, hub_url=hub_url, offline=False, cache_dir=cache_dir))


def inspect(
    model: str = "default",
    *,
    cache_dir: str | os.PathLike[str] | None = None,
) -> ModelInfo:
    """Describes ``model`` as it is available locally, verifying and loading
    it, without any network access.

    Raises:
        ModelUnavailableError: The model is not in the cache.
        ModelIntegrityError: A file fails verification.
    """
    return _info(_load(model, hub_url=None, offline=True, cache_dir=cache_dir))


def _load(
    model: str,
    *,
    hub_url: str | None,
    offline: bool,
    cache_dir: str | os.PathLike[str] | None,
) -> _univo.Model:
    return _univo.load_model(
        model,
        transport=_LazyTransport(),
        hub_url=hub_url,
        offline=offline,
        cache_dir=None if cache_dir is None else os.fspath(cache_dir),
    )


class _LazyTransport:
    """Imports the HTTP stack only when a request is actually made, so that
    loading a cached model stays free of it."""

    def fetch(self, url: str, destination: os.PathLike[str], max_bytes: int) -> None:
        from univo._transport import UrllibTransport

        UrllibTransport().fetch(url, destination, max_bytes)

    def fetch_text(self, url: str, max_bytes: int) -> str:
        from univo._transport import UrllibTransport

        return UrllibTransport().fetch_text(url, max_bytes)

    def keep_waiting(self) -> bool:
        return True


def _info(model: _univo.Model) -> ModelInfo:
    # Imported here: pathlib costs milliseconds at import time.
    import pathlib

    return ModelInfo(
        name=model.name,
        source=cast(Literal["hub", "local"], model.source),
        repository=model.repository,
        revision=model.revision,
        path=pathlib.Path(model.path),
        dimension=model.dimension,
        backend=model.backend,
        files=tuple(ModelFile(name, size, digest) for name, size, digest in model.files),
    )
