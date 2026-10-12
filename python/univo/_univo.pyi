import os
from collections.abc import Iterable
from typing import Literal, Protocol, TypedDict

__version__: str
ERROR_KINDS: list[str]

class Transport(Protocol):
    def fetch(self, url: str, destination: os.PathLike[str], max_bytes: int) -> None: ...
    def fetch_text(self, url: str, max_bytes: int) -> str: ...
    def keep_waiting(self) -> bool: ...

class Model:
    @property
    def name(self) -> str: ...
    @property
    def source(self) -> str: ...
    @property
    def repository(self) -> str | None: ...
    @property
    def revision(self) -> str | None: ...
    @property
    def path(self) -> str: ...
    @property
    def dimension(self) -> int: ...
    @property
    def backend(self) -> str: ...
    @property
    def files(self) -> list[tuple[str, int, str]]: ...
    def encode(self, texts: list[str]) -> tuple[bytes, list[int]]: ...

def load_model(
    model: str,
    *,
    transport: Transport,
    hub_url: str | None = None,
    offline: bool = False,
    cache_dir: str | None = None,
) -> Model: ...

FieldSpec = tuple[str, str, str, float, bool, bool, str]

class Schema:
    def __init__(
        self,
        fields: list[FieldSpec] | None = None,
        id_field: str | None = None,
        partition_by: list[str] = ...,
        limits: tuple[int, int, int] = ...,
    ) -> None: ...
    @property
    def needs_model(self) -> bool: ...
    def configuration_fingerprint(self, model_ref: str, model: Model | None = None) -> str: ...

class RawMatch(TypedDict):
    left_id: str | int
    right_id: str | int
    score: float
    kind: Literal["exact", "candidate"]
    field_scores: dict[str, float | None]
    reasons: list[str]
    conflicts: list[str]
    coverage: float

class RawReport(TypedDict):
    matches: list[RawMatch]
    processed: int
    total_matches: int
    candidates: int
    generation: int
    candidate_truncated: bool
    output_truncated: bool

class Index:
    def __init__(
        self,
        schema: Schema,
        max_records: int,
        model: Model | None = None,
        threads: int = 1,
        cache: EmbeddingCache | None = None,
    ) -> None: ...
    def add(self, inputs: Iterable[object]) -> list[str | int]: ...
    def search(
        self, input: object, min_score: float, limit: int, max_entries: int, exhaustive: bool
    ) -> RawReport: ...
    def find(
        self,
        min_score: float,
        limit: int,
        max_entries: int,
        exhaustive: bool,
        budgets: tuple[int, int],
    ) -> RawReport: ...
    def stats(self) -> dict[str, int]: ...

def validate_model(model: str) -> None: ...
def invalid_option(name: str, expected: str) -> Exception: ...
def validate_threshold(min_score: float) -> float: ...

class EmbeddingCache:
    def __init__(self, model: Model, max_bytes: int) -> None: ...
    def stats(self) -> dict[str, int]: ...
