# Python API

Everything `import univo` exposes. Names not listed here, and names starting with `_`, are private and may change ([stability](api.md#stability)).

```python
from univo import Finder, Schema, Text, Exact                 # build and search
from univo import Match, Report, Group, Groups, Decision      # results
from univo import UnivoError                                  # and one subclass per kind of error
import univo.models                                           # models: download, inspect
```

Record IDs (`RecordId`) are `str` or `int`. Without an `id_field`, records get their position in the input as ID.

## Finder

```python
Finder(*, fields=None, id_field=None, schema=None, model=None, model_path=None,
       hub_url=None, offline=False, cache_dir=None, threads=None, max_records=100_000,
       max_exhaustive_pairs=1_000_000, max_semantic_comparisons=1_000_000,
       embedding_cache_bytes=4 << 20, retrieval="exact", ann_ef_search=512,
       ann_max_candidates=1024)
```

Finds duplicates in a batch (`find`, `group`) and keeps an index of records to search and update (`add`, `upsert`, `remove`, `search`, `check_or_add`). Creating a Finder loads no model and opens no connection. A Finder is safe to share between threads: searches run in parallel, changes one at a time.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `fields` | `None` | Field paths, each compared as `Text()`. A shortcut for a `schema`. Without `fields` or `schema`, each record is one string compared as `Text()`. |
| `id_field` | `None` | With `fields`: the field holding each record's ID. |
| `schema` | `None` | A [`Schema`](#schema). Exclusive of `fields` and `id_field`. |
| `model` | `None` | Embedding model for semantic fields: `"default"`, a Hugging Face repository `"org/name"` or `"org/name@revision"`, or a local path ([models](../design/models.md)). `None` is the default model. |
| `model_path` | `None` | A local model directory. Exclusive of `model`. |
| `hub_url` | `None` | A Hugging Face compatible mirror. Defaults to `HF_ENDPOINT`, then `https://huggingface.co`. |
| `offline` | `False` | Never connect: use only the cache or a local model. |
| `cache_dir` | `None` | Model cache. Defaults to `UNIVO_CACHE_DIR`, then the platform cache directory. |
| `threads` | `None` | Threads for `find`, `group` and encoding. `None` uses every core. |
| `max_records` | `100_000` | Records an index may hold, from 1 to 2³² − 1. Above it: `ResourceLimitError`. |
| `max_exhaustive_pairs` | `1_000_000` | Pairs a `find` or `group` with `exhaustive=True` may compare. |
| `max_semantic_comparisons` | `1_000_000` | Vector comparisons a `find` or `group` with semantic fields may make, with either `retrieval`, unless `exhaustive=True`. It is checked before the work, as a conservative bound: about n² for n records in one partition with `"exact"`, so the default covers about 1,000 records per partition. Above it: `ResourceLimitError`; raise it for larger batches. |
| `embedding_cache_bytes` | `4 << 20` | Memory for recently encoded texts, so repeated texts are encoded once. `0` disables it ([embedding cache](../design/embedding-cache.md)). |
| `retrieval` | `"exact"` | `"exact"` compares every vector of a partition; `"hnsw"` uses an approximate graph ([approximate search](../design/approximate-search.md)). |
| `ann_ef_search` | `512` | With `"hnsw"`: breadth of the graph search, from 1 to 8,192. Higher finds more, slower. |
| `ann_max_candidates` | `1024` | With `"hnsw"`: candidates read per field, from 1 to 100,000. |

### find

```python
finder.find(records, *, min_score=0.8, limit=1000, max_entries=None, exhaustive=False) -> Report
```

Every pair of `records` scoring at least `min_score`, best first. The records are indexed temporarily; the Finder's own index is unchanged.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `records` | | An iterable of records: strings, or dicts matching the schema. |
| `min_score` | `0.8` | Lowest score returned, in `(0, 1]`. |
| `limit` | `1000` | Matches returned. `total_matches` counts all of them. |
| `max_entries` | `None` | Index entries one candidate lane may read per record. Reaching it sets `candidate_truncated`. `None` is unlimited. |
| `exhaustive` | `False` | Compare every pair, without candidate search, within `max_exhaustive_pairs`. The reference for any other mode. |

Raises `InvalidRecordError`, `DuplicateIdError` or `ResourceLimitError` before any result.

### group

```python
finder.group(records, *, min_score=0.8, max_group_size=100, max_pairs=1_000_000,
             max_entries=None, exhaustive=False) -> Groups
```

Groups of records to review together. A record joins a group only if it reaches `min_score` with **every** member, so chains of near matches stay apart ([groups](../design/groups.md)). The Finder's own index is unchanged.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `max_group_size` | `100` | Largest group. |
| `max_pairs` | `1_000_000` | Qualifying pairs held in memory. Above it: `ResourceLimitError`. |

`records`, `min_score`, `max_entries` and `exhaustive` are as in `find`.

### add

```python
finder.add(records) -> list[RecordId]
```

Adds a batch to the index and returns their IDs. The batch is added entirely or not at all. Semantic fields are encoded here. Raises `DuplicateIdError` for an ID already present.

### upsert

```python
finder.upsert(records) -> list[RecordId]
```

Adds records, or replaces those whose ID is present, in one step. Needs an `id_field` ([mutations](../design/mutations.md)).

### remove

```python
finder.remove(ids) -> list[RecordId]
```

Removes the records whose IDs are in `ids`, in one step. Returns the IDs that were not in the index.

### search

```python
finder.search(record, *, limit=10, min_score=0.8, max_entries=None, exhaustive=False) -> Report
```

Indexed records similar to `record`, best first. The index is unchanged. A record with an ID already in the index is not matched with itself. Options are as in `find`.

### check_or_add

```python
finder.check_or_add(record, *, min_score=0.8, limit=10, max_entries=None, exhaustive=False) -> Decision
```

Searches for `record` and adds it only if nothing reaches `min_score`, in one step for this Finder: two threads never both add the same duplicate. It does not coordinate other processes or a database ([concurrency](../design/concurrency.md)). Options are as in `search`.

### save

```python
finder.save(path, *, max_snapshot_bytes=512 << 20) -> None
```

Writes every indexed record, its vectors and the Finder's configuration to a new generation under `path`, then publishes it atomically: a reader sees the previous generation or the new one, never a mix. Changes made afterwards are lost unless you save again.

- **Errors before publication** leave the previous generation current. Above `max_snapshot_bytes`, `ResourceLimitError` is raised before any file is written.
- **Errors after publication**, such as a failed directory sync, can leave the new generation current. After an error, load the snapshot to know which generation it holds.
- **Concurrent writers** of the same path are refused ([snapshots](../design/snapshots.md)).

### load

```python
Finder.load(path, *, offline=False, cache_dir=None, hub_url=None, threads=None,
            max_snapshot_bytes=512 << 20, max_records=100_000) -> Finder
```

A Finder restored from a snapshot, with its schema and settings. Records are not encoded again. A semantic snapshot loads the exact model it was built with, verified, from the cache when `offline`. The index holds at most the smaller of the saved limit and `max_records`: raise it to load more than 100,000 records. `max_snapshot_bytes` bounds the files read. Raises `IndexCompatibilityError` for a snapshot of another format, normalizer, scoring or search version, or model.

### config

```python
finder.config() -> dict
```

The resolved settings: schema, model reference, limits, versions and `config_fingerprint`, which changes when anything affecting results changes. `resolved_model` appears once the model is loaded.

### stats

```python
finder.stats() -> dict[str, int]
```

Index counts (`records`, `retained_slots`, `tombstones`, `generation`), estimated memory (`index_bytes`, `ann_bytes`, `ann_nodes`) and embedding cache counters (`embedding_cache_hits`, `embedding_cache_misses`, `embedding_cache_evictions`, `embedding_cache_bytes`, `embedding_cache_entries`).

## Schema

```python
Schema(fields, id_field=None, partition_by=(), max_fields=64, max_depth=16,
       max_selected_bytes=1 << 20)
```

Which fields to compare and how. Field paths use dots for nested fields: `"address.city"`.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `fields` | | A mapping from field path to [`Text`](#text) or [`Exact`](#exact). |
| `id_field` | `None` | The field holding each record's ID: `str` or `int`. Needed by `upsert` and `remove`. |
| `partition_by` | `()` | Field paths. Records match only when these are equal: tenant, country, customer. Each record must have them. |
| `max_fields` | `64` | Compared fields per record. |
| `max_depth` | `16` | Nesting depth, at most 128. |
| `max_selected_bytes` | `1 << 20` | Bytes of compared values per record. |

Raises `InvalidSchemaError` when inconsistent. Fields outside the schema are ignored, and records are never modified.

## Text

```python
Text(mode="hybrid", weight=1.0, case_fold=True, required=False)
```

A text field.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `mode` | `"hybrid"` | `"lexical"` compares spelling: edit distance and trigrams. `"semantic"` compares meaning with the model. `"hybrid"` is lexical for now. |
| `weight` | `1.0` | Weight of the field in the score. |
| `case_fold` | `True` | Ignore case. |
| `required` | `False` | A record without this field raises `InvalidRecordError`. Otherwise a missing value gives no evidence. |

Text is normalized to Unicode NFC, and case folded unless `case_fold=False`. Whitespace is compared as it is.

## Exact

```python
Exact(normalizer="identity", weight=1.0, required=False, conflict="evidence")
```

A field compared by equality: codes, numbers, emails, booleans, lists, objects.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `normalizer` | `"identity"` | `"hostname"` also lowercases ASCII letters and drops one trailing dot. |
| `weight` | `1.0` | Weight of the field in the score. |
| `required` | `False` | As in `Text`. |
| `conflict` | `"evidence"` | `"reject"` makes a difference a veto: the two records never match, whatever their score. |

Objects are compared key by key; a missing or `None` value gives no evidence ([scoring](scoring.md)).

## Results

### Match

A pair of records and why they match.

| Attribute | Type | Meaning |
| --- | --- | --- |
| `left_id`, `right_id` | `RecordId` | The two records. |
| `score` | `float` | In `[0, 1]`. Ranks pairs; not a probability. |
| `kind` | `"exact"` or `"candidate"` | `"exact"`: equal on every compared field after normalization. |
| `field_scores` | `dict[str, float \| None]` | Similarity per field; `None` when the field gave no evidence. |
| `reasons` | `list[str]` | The signal behind each field's score, such as `"name: trigrams"`. |
| `conflicts` | `list[str]` | Fields that differ. |
| `coverage` | `float` | Share of the schema's weight that gave evidence. A low coverage means few fields were compared. |

### Report

What `find`, `search` and `check_or_add` return.

| Attribute | Type | Meaning |
| --- | --- | --- |
| `matches` | `tuple[Match, ...]` | Best first, up to `limit`. |
| `total_matches` | `int` | All matches found, including those beyond `limit`. |
| `processed` | `int` | Records processed. |
| `candidates` | `int` | Pairs scored. |
| `generation` | `int` | Index version: it increases with every change. |
| `config_fingerprint` | `str` | As in `config()`. |
| `search_profile` | `"exhaustive"`, `"bounded"` or `"approximate"` | How the search ran. |
| `candidate_truncated` | `bool` | A work limit was reached: matches may be missing. |
| `output_truncated` | `bool` | More matches than `limit`. |
| `warnings` | `tuple[str, ...]` | The above, explained. |

### Groups

What `group` returns.

| Attribute | Type | Meaning |
| --- | --- | --- |
| `exact` | `tuple[tuple[RecordId, ...], ...]` | Records equal on every compared field after normalization. |
| `groups` | `tuple[Group, ...]` | Records every pair of which reaches `min_score`. An exact group usually appears here too. |
| `qualifying_pairs` | `int` | Pairs reaching `min_score`. |
| `split_pairs` | `int` | Qualifying pairs whose records ended in different groups. |
| `processed`, `candidates`, `config_fingerprint`, `search_profile`, `candidate_truncated`, `warnings` | | As in `Report`. |

### Group

| Attribute | Type | Meaning |
| --- | --- | --- |
| `members` | `tuple[RecordId, ...]` | In input order. |
| `min_score` | `float` | The lowest score among its pairs. |

### Decision

What `check_or_add` returns.

| Attribute | Type | Meaning |
| --- | --- | --- |
| `status` | `"inserted"`, `"matched"` or `"review"` | `inserted`: nothing reached `min_score`, the record was added. `matched`: an indexed record is equal on every field. `review`: similar records exist; nothing was added. |
| `id` | `RecordId \| None` | The added record's ID, when `inserted`. |
| `report` | `Report` | The matches the decision is based on. |

## Errors

Every error is a `UnivoError` with these attributes:

| Attribute | Meaning |
| --- | --- |
| `code` | Stable identifier of the cause, such as `record.wrong_type`. |
| `hint` | How to fix it. |
| `path` | The field concerned, if any. |
| `details` | Types, sizes and limits. |
| `documentation_url` | The code's entry in the [error catalog](errors.md). |
| `record_index` | Position of the record in a batch, if any. |
| `record_id` | Its ID, when valid. Never printed. |

| Exception | Raised when |
| --- | --- |
| `InvalidSchemaError` | The schema is inconsistent. |
| `InvalidRecordError` | A record does not satisfy the schema. |
| `DuplicateIdError` | An ID is already in the batch or the index. |
| `ResourceLimitError` | A record, batch, index or model exceeds a limit. |
| `SchemaMismatchError` | Records prepared with different schemas meet. |
| `ModelUnavailableError` | The model cannot be found, downloaded or loaded. |
| `ModelIntegrityError` | Model files fail verification. |
| `IndexCompatibilityError` | A snapshot or cache comes from another model, format or version. |
| `InvalidParameterError` | An option is out of range. Also a `ValueError`. |

Messages never contain field values.

## univo.models

Prepare models ahead of time, for instance in a container image.

```python
univo.models.download(model="default", *, hub_url=None, cache_dir=None) -> ModelInfo
univo.models.inspect(model="default", *, cache_dir=None) -> ModelInfo
```

- **`download`** downloads `model` into the cache unless it is there, verifies and loads it. Only repository and file names are sent.
- **`inspect`** describes a model already available locally, verified and loaded, without network access.

`ModelInfo` has `name`, `source` (`"hub"` or `"local"`), `repository`, `revision` (full commit), `path`, `dimension`, `backend` and `files`. Each `ModelFile` has `name`, `size` in bytes and `digest`.

From a shell:

```bash
python -m univo.models download default
```

## Version

`univo.__version__` is the installed version, such as `"1.0.1"`.

## Stability

From 1.0, Univo follows [Semantic Versioning](https://semver.org/). This page says what each 1.x release keeps compatible, and how to upgrade when a release changes scores or formats.

### Kept compatible across 1.x

- **Python API.**
  - **Names.** Every name in `univo.__all__` and in `univo.models`.
  - **Parameters.** Their parameters and defaults.
  - **Results.** The attributes of `Report`, `Match`, `Groups`, `Group` and `Decision`.
  - **Errors.** The exception classes and their hierarchy.
  - **Additions.** A minor release may add keyword parameters with defaults, attributes and exception subclasses.
  - **Removals and renamings.** These wait for 2.0, after a deprecation warning in a 1.x release.
- **Error codes.** The `code` of each error, such as `limit.pair_count`, and its kind. Messages and hints may be reworded.
- **Explanations.** `Match.reasons` holds `"<path>: <signal>"` with a signal of `missing`, `edit_distance`, `trigrams`, `exact`, `keys`, `semantic` or `acronym`, and `"<path>: normalized (<steps>)"` for exact matches. `Match.conflicts` lists the paths of contradicting fields. `search_profile` is `exhaustive`, `bounded` or `approximate`.
- **Completeness.** Without a reported limit (`candidate_truncated`, `output_truncated`) and outside the explicit approximate profile, search returns exactly what the exhaustive oracle returns.
- **Hard rules.** Partitions, vetoes, typed IDs and required fields.

### May change in a minor release

Each change is announced in the [release notes](https://github.com/leopoldch/univo/releases).

- **Scores.** Raising `SCORING_VERSION` changes the scores of some pairs, as version 4 did for names with distinct words. Rankings stay explained; thresholds calibrated on an earlier version should be checked again.
- **The default model.** It is pinned by revision and checksums. A new default model is a minor release. Existing snapshots keep the model they were built with: `Finder.load` uses the pinned model recorded in the snapshot, not the current default.
- **Messages, hints and warning texts.**

### Not covered

- **Internals.** The Rust crates, which are not published, and private modules such as `univo._univo` and `univo._api`.
- **Exact numbers.** Performance figures, which budgets test on a reference machine. `stats()` byte estimates. `config_fingerprint` values.

### Upgrading

1. **Read the release notes** for the versions in between.
2. **Rebuild snapshots that a release refuses.** A snapshot records its format, normalizer, scoring and search versions, its schema, and the identity of its model: repository, revision and file checksums. `Finder.load` refuses it with `IndexCompatibilityError` when one of the four versions differs from the running release, in either direction, or when the model files it loads do not match the recorded identity. There is no silent migration. These versions change independently of the package version: a snapshot can survive an upgrade, and an older release loads a newer snapshot when they all match, unless the snapshot uses a `Finder` option the older release lacks (`InvalidParameterError`, code `parameter.invalid_option`). Snapshots keep only normalized selected fields, so rebuild from the original records: create the `Finder` with the same schema, `add` the records, then `save`.
3. **Keep the snapshot's model available.** Loading a semantic snapshot needs its recorded model, in the cache when offline, even after the default model changed.
4. **Check calibrated thresholds** when `SCORING_VERSION` changed, on the same labelled data as before.
