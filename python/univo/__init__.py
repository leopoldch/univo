"""Fast, local duplicate detection for your data."""

from univo._univo import __version__
from univo.errors import (
    DuplicateIdError,
    InvalidRecordError,
    InvalidSchemaError,
    ModelIntegrityError,
    ModelUnavailableError,
    ResourceLimitError,
    SchemaMismatchError,
    UnivoError,
)

__all__ = [
    "DuplicateIdError",
    "InvalidRecordError",
    "InvalidSchemaError",
    "ModelIntegrityError",
    "ModelUnavailableError",
    "ResourceLimitError",
    "SchemaMismatchError",
    "UnivoError",
    "__version__",
]
