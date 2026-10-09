"""Fast, local duplicate detection for your data."""

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
    "__version__",
]
