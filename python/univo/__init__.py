"""Fast, local duplicate detection for your data."""

from typing import TYPE_CHECKING

from univo._univo import __version__
from univo.errors import (
    DuplicateIdError,
    IndexCompatibilityError,
    InvalidParameterError,
    InvalidRecordError,
    InvalidSchemaError,
    ModelIntegrityError,
    ModelUnavailableError,
    ResourceLimitError,
    SchemaMismatchError,
    UnivoError,
)

__all__ = [
    "Decision",
    "DuplicateIdError",
    "Exact",
    "Finder",
    "IndexCompatibilityError",
    "InvalidParameterError",
    "InvalidRecordError",
    "InvalidSchemaError",
    "Match",
    "ModelIntegrityError",
    "ModelUnavailableError",
    "Report",
    "ResourceLimitError",
    "Schema",
    "SchemaMismatchError",
    "Text",
    "UnivoError",
    "__version__",
]


def __getattr__(name: str) -> object:
    if name in {"Decision", "Finder", "Schema", "Text", "Exact", "Match", "Report"}:
        from univo import _api

        value = getattr(_api, name)
        globals()[name] = value
        return value
    raise AttributeError(f"module univo has no attribute {name!r}")


if TYPE_CHECKING:
    from univo._api import Decision, Exact, Finder, Match, Report, Schema, Text
