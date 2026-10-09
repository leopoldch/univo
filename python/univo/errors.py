"""Exceptions raised by Univo.

Every error comes from the Rust core with the same parts in every language;
see the error catalog for each code. Field values and record identifiers
are never part of the message.
"""

from collections.abc import Mapping

__all__ = [
    "DuplicateIdError",
    "IndexCompatibilityError",
    "InvalidParameterError",
    "InvalidRecordError",
    "InvalidSchemaError",
    "ModelIntegrityError",
    "ModelUnavailableError",
    "ResourceLimitError",
    "SchemaMismatchError",
    "UnivoError",
]


class UnivoError(Exception):
    """Base of every error raised by Univo.

    Attributes:
        code: Stable identifier of the cause, such as ``record.wrong_type``.
        hint: How to fix it.
        path: Field concerned, if any.
        details: Structured context: types, sizes and limits.
        documentation_url: Entry of the code in the error catalog.
        record_index: Position of the record in a batch, if any.
        record_id: Identifier of that record, when valid; never printed.
    """

    code: str
    hint: str
    path: str | None
    details: dict[str, str | int | float]
    documentation_url: str
    record_index: int | None
    record_id: str | int | None

    def __init__(
        self,
        message: str,
        *,
        code: str,
        hint: str,
        path: str | None = None,
        details: Mapping[str, str | int | float] | None = None,
        documentation_url: str = "",
        record_index: int | None = None,
        record_id: str | int | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.hint = hint
        self.path = path
        self.details = dict(details or {})
        self.documentation_url = documentation_url
        self.record_index = record_index
        self.record_id = record_id


class InvalidSchemaError(UnivoError):
    """The schema is inconsistent; nothing was compared."""


class InvalidRecordError(UnivoError):
    """A record does not satisfy the schema."""


class DuplicateIdError(UnivoError):
    """An identifier already exists in a batch or index."""


class ResourceLimitError(UnivoError):
    """A record, batch or model exceeds a configured resource limit."""


class SchemaMismatchError(UnivoError):
    """Records prepared with different schemas were compared."""


class ModelUnavailableError(UnivoError):
    """The embedding model cannot be resolved, downloaded or loaded."""


class ModelIntegrityError(UnivoError):
    """Model files or outputs fail verification."""


class IndexCompatibilityError(UnivoError):
    """Stored state, such as embeddings, comes from another model or format
    than the one in use, or an index is used with records it does not
    cover."""


class InvalidParameterError(UnivoError, ValueError):
    """A parameter of a call is outside its valid range."""
