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
    search_profile: Literal["exhaustive", "bounded", "approximate"]
    candidate_truncated: bool
    output_truncated: bool
    warnings: tuple[str, ...]


@dataclass(frozen=True)
class Decision:
    """Outcome of ``Finder.check_or_add``.

    ``status`` is ``"inserted"`` when no stored record reached ``min_score``,
    ``"matched"`` when a stored record is equal on every selected field after
    normalization, and ``"review"`` when records reach ``min_score`` without
    being equal. ``id`` is the inserted record's ID, ``None`` otherwise.
    ``report`` holds the matches the decision was based on.
    """

    status: Literal["inserted", "matched", "review"]
    id: RecordId | None
    report: Report


class Finder:
    """Local matching. Defaults use lexical hybrid evidence (policy generic-v1).

    ``min_score`` ranks candidates, without a calibrated identity decision.
    ``find`` uses a temporary index. ``add``, ``upsert``, ``remove`` and
    ``check_or_add`` change the stored index. A Finder is safe to share
    between threads: searches run in parallel, changes one at a time.
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
        retrieval: Literal["exact", "hnsw"] = "exact",
        ann_ef_search: int = 512,
        ann_max_candidates: int = 1024,
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
        if not isinstance(retrieval, str) or retrieval not in ("exact", "hnsw"):
            raise _univo.invalid_option("retrieval", "exact or hnsw")
        for name, value, maximum in (
            ("ann_ef_search", ann_ef_search, 8192),
            ("ann_max_candidates", ann_max_candidates, 100_000),
        ):
            if type(value) is not int or not 1 <= value <= maximum:
                raise _univo.invalid_option(name, f"an integer in [1, {maximum}]")
        self._retrieval = retrieval
        self._ann_ef_search = ann_ef_search
        self._ann_max_candidates = ann_max_candidates
        self._ann = (ann_ef_search, ann_max_candidates) if retrieval == "hnsw" else None
        self._embedding_cache_bytes = embedding_cache_bytes
        self._embedding_cache: _univo.EmbeddingCache | None = None
        self._max_semantic_comparisons = max_semantic_comparisons
        self._max_exhaustive_pairs = max_exhaustive_pairs
        self._schema = schema
        self._native_schema = _univo.Schema() if schema is None else schema._native
        self._resolved_ref: str | None = None
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

    def _resolve_model(self) -> None:
        if self._native_schema.needs_model and self._model is None:
            from univo.models import _load

            self._model = _load(
                self._resolved_ref or self._model_ref,
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

    def _new_index(self, *, exhaustive: bool = False) -> _univo.Index:
        self._resolve_model()
        return _univo.Index(
            self._native_schema,
            self._max_records,
            self._model,
            self._threads,
            self._embedding_cache,
            None if exhaustive else self._ann,
        )

    def _persistent_index(self) -> _univo.Index:
        """The stored index, created on first use. Native calls on it run
        outside ``self._lock``: the index orders them itself."""
        with self._lock:
            if self._index is None:
                self._index = self._new_index()
            return self._index

    def add(self, records: Iterable[object]) -> list[RecordId]:
        """Validate and encode a bounded batch, then atomically append it."""
        return self._persistent_index().add(records)

    def upsert(self, records: Iterable[object]) -> list[RecordId]:
        """Atomically replace or insert a bounded batch by typed external ID."""
        self._native_schema.check_mutation("upsert")
        return self._persistent_index().upsert(records)

    def remove(self, ids: Iterable[RecordId]) -> list[RecordId]:
        """Remove active IDs atomically; return IDs absent at batch start."""
        with self._lock:
            index = self._index
        if index is None:
            return self._native_schema.missing_ids(ids, self._max_records)
        return index.remove(ids)

    def check_or_add(
        self,
        record: object,
        *,
        min_score: float = 0.8,
        limit: int = 10,
        max_entries: int | None = None,
        exhaustive: bool = False,
    ) -> Decision:
        """Search for ``record`` and add it only if nothing reaches ``min_score``.

        The search and the insertion are one atomic step for this Finder:
        concurrent calls in this process never both insert a duplicate. It
        does not coordinate other processes or an external database, and a
        bounded search without matches is not a proof that none exist.
        """
        min_score = self._validate_options(min_score, limit, max_entries)
        outcome = self._persistent_index().check_or_add(
            record, min_score, limit, self._entries(max_entries), exhaustive
        )
        return Decision(
            status=outcome["decision"],
            id=outcome["id"],
            report=self._report(outcome["report"], exhaustive),
        )

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
            index = self._new_index(exhaustive=exhaustive)
            stored = self._index
        budget = self._max_semantic_comparisons if self._ann and not exhaustive else None
        index.add(records, budget)
        result = index.find(
            min_score,
            limit,
            self._entries(max_entries),
            exhaustive,
            (self._max_exhaustive_pairs, self._max_semantic_comparisons),
        )
        result["generation"] = 0 if stored is None else stored.stats()["generation"]
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
        if result["approximate"]:
            warnings.append(
                "HNSW approximate retrieval; matches may be missing even without saturation."
            )
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
                self._model_ref, self._model, self._ann
            ),
            search_profile="exhaustive"
            if exhaustive
            else ("approximate" if result["approximate"] else "bounded"),
            candidate_truncated=result["candidate_truncated"],
            output_truncated=result["output_truncated"],
            warnings=tuple(warnings),
        )

    @staticmethod
    def _snapshot_budget(value: int) -> None:
        if type(value) is not int or not 0 < value <= 2**63 - 1:
            raise _univo.invalid_option("max_snapshot_bytes", "an integer in [1, 2**63 - 1]")

    def save(self, path: str | os.PathLike[str], *, max_snapshot_bytes: int = 512 << 20) -> None:
        """Publish a complete local snapshot; concurrent writers are refused."""
        import json
        from dataclasses import asdict

        self._snapshot_budget(max_snapshot_bytes)
        # Reject giant schema/reference strings before JSON copies or model I/O.
        metadata_chars = len(self._model_ref)
        if self._schema is not None:
            metadata_chars += 128 * len(self._schema.fields)
            for field_path in self._schema.fields:
                metadata_chars += len(field_path)
            metadata_chars += sum(len(path) for path in self._schema.partition_by)
            metadata_chars += len(self._schema.id_field or "")
        if metadata_chars > 1 << 20:
            raise _univo.invalid_option(
                "snapshot_metadata", "shorter field paths and model references"
            )
        index = self._persistent_index()
        with self._lock:
            schema = None
            if self._schema is not None:
                schema = {
                    "fields": [
                        {
                            "path": path,
                            "kind": "text" if isinstance(options, Text) else "exact",
                            "options": asdict(options),
                        }
                        for path, options in self._schema.fields.items()
                    ],
                    "id_field": self._schema.id_field,
                    "partition_by": list(self._schema.partition_by),
                    "max_fields": self._schema.max_fields,
                    "max_depth": self._schema.max_depth,
                    "max_selected_bytes": self._schema.max_selected_bytes,
                }
            resolved = None
            if self._model is not None:
                resolved = (
                    f"{self._model.repository}@{self._model.revision}"
                    if self._model.repository is not None
                    else os.fspath(self._model.path)
                )
            config = {
                "schema": schema,
                "model": self._model_ref,
                "resolved_model": resolved,
                "max_records": self._max_records,
                "max_exhaustive_pairs": self._max_exhaustive_pairs,
                "max_semantic_comparisons": self._max_semantic_comparisons,
                "embedding_cache_bytes": self._embedding_cache_bytes,
            }
            if self._ann is not None:
                config.update(
                    retrieval=self._retrieval,
                    ann_ef_search=self._ann_ef_search,
                    ann_max_candidates=self._ann_max_candidates,
                )
        index.save(os.fspath(path), json.dumps(config, allow_nan=False), max_snapshot_bytes)

    @classmethod
    def load(
        cls,
        path: str | os.PathLike[str],
        *,
        offline: bool = False,
        cache_dir: str | os.PathLike[str] | None = None,
        hub_url: str | None = None,
        threads: int = 1,
        max_snapshot_bytes: int = 512 << 20,
        max_records: int = 100_000,
    ) -> Finder:
        """Restore selected fields and vectors; verify the pinned local model.

        Semantic snapshots require compatible model files, cached when offline.
        This explicit call may load a model. It never reencodes saved records.
        """
        import json

        cls._snapshot_budget(max_snapshot_bytes)
        if type(max_records) is not int or not 0 < max_records <= 2**32 - 1:
            raise _univo.invalid_option("max_records", "an integer in [1, 2**32 - 1]")
        snapshot = _univo.Snapshot(os.fspath(path), max_snapshot_bytes)
        config = json.loads(snapshot.configuration)
        try:
            saved_schema = config.pop("schema")
            resolved = config.pop("resolved_model")
            schema = None
            if saved_schema is not None:
                fields = {}
                for item in saved_schema.pop("fields"):
                    kind = item["kind"]
                    if kind not in ("text", "exact") or item["path"] in fields:
                        raise ValueError("invalid fields")
                    fields[item["path"]] = (Text if kind == "text" else Exact)(**item["options"])
                schema = Schema(fields=fields, **saved_schema)
            finder = cls(
                schema=schema,
                offline=offline,
                cache_dir=cache_dir,
                hub_url=hub_url,
                threads=threads,
                **config,
            )
            if resolved is not None and not isinstance(resolved, str):
                raise ValueError("invalid model reference")
            finder._max_records = min(finder._max_records, max_records)
            finder._resolved_ref = resolved
        except (KeyError, TypeError, ValueError, AttributeError):
            raise _univo.invalid_option("snapshot", "a supported Finder configuration") from None
        if finder._native_schema.needs_model and resolved is None:
            raise _univo.invalid_option("snapshot", "a pinned semantic model")
        snapshot.check_capacity(finder._max_records)
        finder._resolve_model()
        finder._index = snapshot.restore(
            finder._native_schema,
            finder._max_records,
            finder._model,
            threads,
            finder._embedding_cache,
            finder._ann,
        )
        return finder

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
                "retrieval": self._retrieval,
                "ann_ef_search": self._ann_ef_search,
                "ann_max_candidates": self._ann_max_candidates,
                "config_fingerprint": self._native_schema.configuration_fingerprint(
                    self._model_ref, self._model, self._ann
                ),
            }

    def stats(self) -> dict[str, int]:
        """Index counts and separate cache diagnostics; byte values are estimates."""
        with self._lock:
            index, cache = self._index, self._embedding_cache
        stats = (
            {
                "records": 0,
                "retained_slots": 0,
                "tombstones": 0,
                "generation": 0,
                "index_bytes": 0,
                "ann_bytes": 0,
                "ann_nodes": 0,
            }
            if index is None
            else index.stats()
        )
        stats.update(
            cache.stats()
            if cache is not None
            else {
                "embedding_cache_hits": 0,
                "embedding_cache_misses": 0,
                "embedding_cache_evictions": 0,
                "embedding_cache_bytes": 0,
                "embedding_cache_entries": 0,
            }
        )
        return stats
