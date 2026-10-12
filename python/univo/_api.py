"""Python input/output conveniences around the shared Rust engine."""

from __future__ import annotations

import os
import threading
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass, field
from typing import Literal

from univo import _univo

RecordId = str | int


@dataclass(frozen=True)
class Text:
    mode: Literal["hybrid", "lexical", "semantic"] = "hybrid"
    weight: float = 1.0
    case_fold: bool = True
    required: bool = False


@dataclass(frozen=True)
class Exact:
    normalizer: Literal["identity", "hostname"] = "identity"
    weight: float = 1.0
    required: bool = False
    conflict: Literal["evidence", "reject"] = "evidence"


@dataclass(frozen=True)
class Schema:
    fields: Mapping[str, Text | Exact]
    id_field: str | None = None
    partition_by: Sequence[str] = ()
    max_fields: int = 64
    max_depth: int = 16
    max_selected_bytes: int = 1 << 20
    _native: _univo.Schema = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        from types import MappingProxyType

        fields = dict(self.fields)
        specs: list[_univo.FieldSpec] = []
        for path, options in fields.items():
            if isinstance(options, Text):
                specs.append(
                    (
                        path,
                        "text",
                        options.mode,
                        options.weight,
                        options.required,
                        options.case_fold,
                        "evidence",
                    )
                )
            elif isinstance(options, Exact):
                specs.append(
                    (
                        path,
                        "exact",
                        options.normalizer,
                        options.weight,
                        options.required,
                        False,
                        options.conflict,
                    )
                )
            else:
                raise _univo.invalid_option("fields", "Text or Exact fields")
        native = _univo.Schema(
            specs,
            self.id_field,
            list(self.partition_by),
            (self.max_fields, self.max_depth, self.max_selected_bytes),
        )
        object.__setattr__(self, "fields", MappingProxyType(fields))
        object.__setattr__(self, "partition_by", tuple(self.partition_by))
        object.__setattr__(self, "_native", native)


@dataclass(frozen=True)
class Match:
    left_id: RecordId
    right_id: RecordId
    score: float
    kind: Literal["exact", "candidate"]
    field_scores: dict[str, float | None]
    reasons: list[str]
    conflicts: list[str]
    coverage: float


@dataclass(frozen=True)
class Report:
    matches: tuple[Match, ...]
    processed: int
    total_matches: int
    candidates: int
    generation: int
    config_fingerprint: str
    search_profile: Literal["exhaustive", "bounded"]
    candidate_truncated: bool
    output_truncated: bool
    warnings: tuple[str, ...]


class Finder:
    """Local matching. Defaults use lexical hybrid evidence (policy generic-v1).

    ``min_score`` ranks candidates, without a calibrated identity decision.
    ``find`` uses a temporary index. Only ``add`` changes the stored index.
    """

    def __init__(
        self,
        *,
        fields: Sequence[str] | None = None,
        id_field: str | None = None,
        schema: Schema | None = None,
        model: str | None = None,
        model_path: str | os.PathLike[str] | None = None,
        hub_url: str | None = None,
        offline: bool = False,
        cache_dir: str | os.PathLike[str] | None = None,
        threads: int = 1,
        max_records: int = 100_000,
        max_exhaustive_pairs: int = 1_000_000,
        max_semantic_comparisons: int = 1_000_000,
        embedding_cache_bytes: int = 4 << 20,
    ) -> None:
        if schema is not None and (fields is not None or id_field is not None):
            raise _univo.invalid_option("schema", "exclusive of fields and id_field")
        if fields is None and id_field is not None and schema is None:
            raise _univo.invalid_option("id_field", "used with fields or schema")
        if model is not None and model_path is not None:
            raise _univo.invalid_option("model", "exclusive of model_path")
        if type(threads) is not int or threads < 1:
            raise _univo.invalid_option("threads", "a positive integer")
        if type(max_records) is not int or not 0 < max_records <= 2**32 - 1:
            raise _univo.invalid_option("max_records", "an integer in [1, 2**32 - 1]")
        if fields is not None:
            if isinstance(fields, str) or len(set(fields)) != len(fields):
                raise _univo.invalid_option("fields", "a sequence of distinct paths")
            schema = Schema(fields={path: Text() for path in fields}, id_field=id_field)
        if type(max_exhaustive_pairs) is not int or not 0 <= max_exhaustive_pairs <= 2**63 - 1:
            raise _univo.invalid_option("max_exhaustive_pairs", "an integer in [0, 2**63 - 1]")
        if (
            type(max_semantic_comparisons) is not int
            or not 0 <= max_semantic_comparisons <= 2**63 - 1
        ):
            raise _univo.invalid_option("max_semantic_comparisons", "an integer in [0, 2**63 - 1]")
        if type(embedding_cache_bytes) is not int or not 0 <= embedding_cache_bytes <= 2**63 - 1:
            raise _univo.invalid_option("embedding_cache_bytes", "an integer in [0, 2**63 - 1]")
        self._embedding_cache_bytes = embedding_cache_bytes
        self._embedding_cache: _univo.EmbeddingCache | None = None
        self._max_semantic_comparisons = max_semantic_comparisons
        self._max_exhaustive_pairs = max_exhaustive_pairs
        self._schema = schema
        self._native_schema = _univo.Schema() if schema is None else schema._native
        self._model_ref = "default" if model is None else model
        if model_path is not None:
            path = os.fspath(model_path)
            self._model_ref = path if path.startswith("file://") else os.path.abspath(path)
        _univo.validate_model(self._model_ref)
        self._hub_url = hub_url
        self._offline = offline
        self._cache_dir = None if cache_dir is None else os.fspath(cache_dir)
        self._threads = threads
        self._max_records = max_records
        self._model: _univo.Model | None = None
        self._index: _univo.Index | None = None
        self._lock = threading.RLock()

    def _new_index(self) -> _univo.Index:
        if self._native_schema.needs_model and self._model is None:
            from univo.models import _load

            self._model = _load(
                self._model_ref,
                hub_url=self._hub_url,
                offline=self._offline,
                cache_dir=self._cache_dir,
            )
        if (
            self._native_schema.needs_model
            and self._embedding_cache_bytes
            and self._embedding_cache is None
        ):
            assert self._model is not None
            self._embedding_cache = _univo.EmbeddingCache(self._model, self._embedding_cache_bytes)
        return _univo.Index(
            self._native_schema,
            self._max_records,
            self._model,
            self._threads,
            self._embedding_cache,
        )

    def _persistent_index(self) -> _univo.Index:
        if self._index is None:
            self._index = self._new_index()
        return self._index

    def add(self, records: Iterable[object]) -> list[RecordId]:
        """Validate and encode a bounded batch, then atomically append it."""
        with self._lock:
            return self._persistent_index().add(records)

    def find(
        self,
        records: Iterable[object],
        *,
        min_score: float = 0.8,
        limit: int = 1000,
        max_entries: int | None = None,
        exhaustive: bool = False,
    ) -> Report:
        """Report unordered pairs from a temporary dataset, without storing it."""
        min_score = self._validate_options(min_score, limit, max_entries)
        with self._lock:
            index = self._new_index()
            index.add(records)
            result = index.find(
                min_score,
                limit,
                self._entries(max_entries),
                exhaustive,
                (self._max_exhaustive_pairs, self._max_semantic_comparisons),
            )
            result["generation"] = 0 if self._index is None else self._index.stats()["generation"]
            return self._report(result, exhaustive)

    def search(
        self,
        record: object,
        limit: int = 10,
        *,
        min_score: float = 0.8,
        max_entries: int | None = None,
        exhaustive: bool = False,
    ) -> Report:
        """Read-only search; exclude an existing query ID. Never merges data."""
        min_score = self._validate_options(min_score, limit, max_entries)
        with self._lock:
            result = self._persistent_index().search(
                record, min_score, limit, self._entries(max_entries), exhaustive
            )
            return self._report(result, exhaustive)

    @staticmethod
    def _validate_options(min_score: float, limit: int, max_entries: int | None) -> float:
        for name, value in (("limit", limit), ("max_entries", max_entries)):
            if value is not None and (type(value) is not int or not 0 <= value <= 2**32 - 1):
                raise _univo.invalid_option(name, "an integer in [0, 2**32 - 1]")

        return _univo.validate_threshold(min_score)

    @staticmethod
    def _entries(value: int | None) -> int:
        return 2**32 - 1 if value is None else value

    def _report(self, result: _univo.RawReport, exhaustive: bool) -> Report:
        warnings = []
        if result["candidate_truncated"]:
            warnings.append("Candidate work limit reached; matches may be missing.")
        if result["output_truncated"]:
            warnings.append("Output limit reached; total_matches includes omitted matches.")
        return Report(
            matches=tuple(Match(**match) for match in result["matches"]),
            processed=result["processed"],
            total_matches=result["total_matches"],
            candidates=result["candidates"],
            generation=result["generation"],
            config_fingerprint=self._native_schema.configuration_fingerprint(
                self._model_ref, self._model
            ),
            search_profile="exhaustive" if exhaustive else "bounded",
            candidate_truncated=result["candidate_truncated"],
            output_truncated=result["output_truncated"],
            warnings=tuple(warnings),
        )

    def config(self) -> dict[str, object]:
        """Resolved schema and versions; model revision appears after lazy loading."""
        from dataclasses import asdict

        with self._lock:
            return {
                "policy": "generic-v1",
                "scoring_version": 2,
                "search_version": 1,
                "normalizer_version": 1,
                "fields": {"<record>": asdict(Text())}
                if self._schema is None
                else {path: asdict(options) for path, options in self._schema.fields.items()},
                "id_field": None if self._schema is None else self._schema.id_field,
                "partition_by": () if self._schema is None else self._schema.partition_by,
                "model": self._model_ref,
                "model_loaded": self._model is not None,
                "resolved_model": None if self._model is None else self._model.name,
                "offline": self._offline,
                "threads": self._threads,
                "effective_threads": self._threads if self._native_schema.needs_model else 1,
                "max_records": self._max_records,
                "max_exhaustive_pairs": self._max_exhaustive_pairs,
                "max_semantic_comparisons": self._max_semantic_comparisons,
                "embedding_cache_bytes": self._embedding_cache_bytes,
                "config_fingerprint": self._native_schema.configuration_fingerprint(
                    self._model_ref, self._model
                ),
            }

    def stats(self) -> dict[str, int]:
        """Index counts and separate cache diagnostics; byte values are estimates."""
        with self._lock:
            stats = (
                {"records": 0, "generation": 0, "index_bytes": 0}
                if self._index is None
                else self._index.stats()
            )
            stats.update(
                self._embedding_cache.stats()
                if self._embedding_cache is not None
                else {
                    "embedding_cache_hits": 0,
                    "embedding_cache_misses": 0,
                    "embedding_cache_evictions": 0,
                    "embedding_cache_bytes": 0,
                    "embedding_cache_entries": 0,
                }
            )
            return stats
