# Errors

Every error raised by Univo, in any language, comes from the Rust core with the same content:

| Part | Content | Python | Rust |
| --- | --- | --- | --- |
| Kind | Category of the error | exception class, such as `univo.InvalidRecordError` | `Error::kind()` |
| Code | Stable identifier of the precise cause, such as `record.wrong_type` | `error.code` | `Error::code()` |
| Message | What happened, naming the field path | `str(error)`, first line | `Error::message()` |
| Hint | How to fix it | `error.hint` | `Error::hint()` |
| Path | Field concerned, if any | `error.path` | `Error::path()` |
| Details | Structured context: expected and found types, limits, sizes | `error.details` (dict) | `Error::details()` |
| Record | Position in a batch, and identifier when valid | `error.record_index`, `error.record_id` | `Error::record()` |

The text of an error has three lines: the message, prefixed with the record position in a batch; the hint; and the code with a link to its entry below.

## Policies

- **No field values in messages.** Messages and details name paths, types, sizes and limits, never the content of a field: errors end up in logs, and record contents must not. Record identifiers are available as an attribute but are not printed.
- **Stable codes.** A released code keeps its meaning. A new cause gets a new code; a retired code is never reused.
- **One source of truth.** Kinds, codes, messages and hints are defined in `crates/univo-core/src/error.rs`. Bindings map each kind to one exception type and copy the other parts; they never compose messages.
- **Exception hierarchy.** Every binding exposes a common base (`univo.UnivoError` in Python) and one subtype per kind, named after the kind with the language's suffix (`Error` in Python, `Exception` in Java).

## Kinds

| Kind | Raised when |
| --- | --- |
| `InvalidSchema` | The schema is inconsistent; nothing was compared. |
| `InvalidRecord` | A record does not satisfy the schema. |
| `ResourceLimit` | A record exceeds a configured resource limit. |
| `SchemaMismatch` | Records prepared with different schemas were compared. |
| `ModelUnavailable` | The embedding model cannot be resolved, downloaded or loaded. |
| `ModelIntegrity` | Model files or outputs fail verification. |
| `IndexCompatibility` | Stored state, such as embeddings, comes from another model or format than the one in use, or an index is used with records it does not cover. |
| `InvalidParameter` | A parameter of a call is outside its valid range; in Python, also a `ValueError`. |

## Catalog

Generated from the Rust core by `crates/univo-core/tests/error_catalog.rs`; each example is produced by a real call.

<!-- catalog:start -->
### state.snapshot_invalid

A snapshot is incomplete, corrupt or incompatible.

- Kind: `IndexCompatibility` (Python: `univo.IndexCompatibilityError`)
- Fix: Restore a complete compatible snapshot or rebuild the index from the source records.

Example:

```text
snapshot rejected: checksum mismatch
hint: Restore a complete compatible snapshot or rebuild the index from the source records.
code: state.snapshot_invalid (https://github.com/leopoldch/univo/blob/main/docs/errors.md#statesnapshot_invalid)
```

### state.snapshot_io

A snapshot operation could not complete.

- Kind: `IndexCompatibility` (Python: `univo.IndexCompatibilityError`)
- Fix: Check local storage permissions and free space, then retry the operation.

Example:

```text
snapshot I/O failed: permission denied
hint: Check local storage permissions and free space, then retry the operation.
code: state.snapshot_io (https://github.com/leopoldch/univo/blob/main/docs/errors.md#statesnapshot_io)
```

### state.snapshot_locked

Another writer holds the snapshot lock.

- Kind: `IndexCompatibility` (Python: `univo.IndexCompatibilityError`)
- Fix: Wait for the other writer to finish before saving to this path.

Example:

```text
another snapshot writer holds this path
hint: Wait for the other writer to finish before saving to this path.
code: state.snapshot_locked (https://github.com/leopoldch/univo/blob/main/docs/errors.md#statesnapshot_locked)
```

### limit.snapshot_bytes

A snapshot exceeds its declared byte budget.

- Kind: `ResourceLimit` (Python: `univo.ResourceLimitError`)
- Fix: Use a smaller index or explicitly raise max_snapshot_bytes.

Example:

```text
snapshot exceeds max_snapshot_bytes
hint: Use a smaller index or explicitly raise max_snapshot_bytes.
code: limit.snapshot_bytes (https://github.com/leopoldch/univo/blob/main/docs/errors.md#limitsnapshot_bytes)
```

### limit.semantic_comparisons

A semantic candidate report exceeds its scan budget.

- Kind: `ResourceLimit` (Python: `univo.ResourceLimitError`)
- Fix: Partition the records or explicitly raise max_semantic_comparisons.

Example:

```text
semantic candidate report requires at most 1000001 vector entries, above the limit of 1000000
hint: Partition the records or explicitly raise max_semantic_comparisons.
code: limit.semantic_comparisons (https://github.com/leopoldch/univo/blob/main/docs/errors.md#limitsemantic_comparisons)
```

### limit.pair_count

An exhaustive report exceeds its pair budget.

- Kind: `ResourceLimit` (Python: `univo.ResourceLimitError`)
- Fix: Use candidate search, partition the records, or explicitly raise max_exhaustive_pairs.

Example:

```text
exhaustive report requires at most 1000001 pairs, above the limit of 1000000
hint: Use candidate search, partition the records, or explicitly raise max_exhaustive_pairs.
code: limit.pair_count (https://github.com/leopoldch/univo/blob/main/docs/errors.md#limitpair_count)
```

### parameter.invalid_option

An API option is invalid.

- Kind: `InvalidParameter` (Python: `univo.InvalidParameterError`)
- Fix: Use one of the documented values for this option.

Example:

```text
option `mode` must be lexical, semantic or hybrid
hint: Use one of the documented values for this option.
code: parameter.invalid_option (https://github.com/leopoldch/univo/blob/main/docs/errors.md#parameterinvalid_option)
```

### schema.no_fields

The schema compares no field.

- Kind: `InvalidSchema` (Python: `univo.InvalidSchemaError`)
- Fix: Add at least one Text or Exact field to the schema.

Example:

```text
the schema selects no field to compare
hint: Add at least one Text or Exact field to the schema.
code: schema.no_fields (https://github.com/leopoldch/univo/blob/main/docs/errors.md#schemano_fields)
```

### schema.too_many_fields

The schema compares more fields than allowed.

- Kind: `InvalidSchema` (Python: `univo.InvalidSchemaError`)
- Fix: Select fewer fields, or raise Limits.max_fields if every field is needed.

Example:

```text
the schema selects 65 fields, above the limit of 64
hint: Select fewer fields, or raise Limits.max_fields if every field is needed.
code: schema.too_many_fields (https://github.com/leopoldch/univo/blob/main/docs/errors.md#schematoo_many_fields)
```

### schema.invalid_weight

A field weight is not a positive finite number.

- Kind: `InvalidSchema` (Python: `univo.InvalidSchemaError`)
- Fix: Use a finite weight greater than zero, such as 1.0.

Example:

```text
field `name` has weight 0, which is not a positive finite number
hint: Use a finite weight greater than zero, such as 1.0.
code: schema.invalid_weight (https://github.com/leopoldch/univo/blob/main/docs/errors.md#schemainvalid_weight)
```

### schema.invalid_limit

A resource limit is out of range.

- Kind: `InvalidSchema` (Python: `univo.InvalidSchemaError`)
- Fix: Use limits greater than zero; max_depth cannot exceed 128.

Example:

```text
limit max_depth is 500, but it cannot exceed 128, which bounds the stack used by normalization
hint: Use limits greater than zero; max_depth cannot exceed 128.
code: schema.invalid_limit (https://github.com/leopoldch/univo/blob/main/docs/errors.md#schemainvalid_limit)
```

### schema.invalid_path

A field path is empty or contains an empty key.

- Kind: `InvalidSchema` (Python: `univo.InvalidSchemaError`)
- Fix: Write paths as non-empty dotted keys such as "params.retries"; build the path from explicit keys when a key contains a dot.

Example:

```text
field path "params..retries" is invalid: a key is empty
hint: Write paths as non-empty dotted keys such as "params.retries"; build the path from explicit keys when a key contains a dot.
code: schema.invalid_path (https://github.com/leopoldch/univo/blob/main/docs/errors.md#schemainvalid_path)
```

### schema.overlapping_paths

Two paths of the schema designate the same or nested values.

- Kind: `InvalidSchema` (Python: `univo.InvalidSchemaError`)
- Fix: Make the identifier, the partition keys and the compared fields designate distinct values; compare either an object or its members, not both.

Example:

```text
compared field `params.retries` overlaps with compared field `params`
hint: Make the identifier, the partition keys and the compared fields designate distinct values; compare either an object or its members, not both.
code: schema.overlapping_paths (https://github.com/leopoldch/univo/blob/main/docs/errors.md#schemaoverlapping_paths)
```

### schema.whole_record_field

A field designates the whole record in an object schema.

- Kind: `InvalidSchema` (Python: `univo.InvalidSchemaError`)
- Fix: Use Schema::strings() to compare plain strings, or give every field a path.

Example:

```text
a field designates the whole record in a schema of objects
hint: Use Schema::strings() to compare plain strings, or give every field a path.
code: schema.whole_record_field (https://github.com/leopoldch/univo/blob/main/docs/errors.md#schemawhole_record_field)
```

### record.wrong_shape

The record, or a value on the way to a field, has the wrong shape.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Pass an object (a dict in Python) whose keys follow the schema paths, or a string for a strings schema.

Example:

```text
the record must be an object, found a string
hint: Pass an object (a dict in Python) whose keys follow the schema paths, or a string for a strings schema.
code: record.wrong_shape (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordwrong_shape)
```

### record.wrong_type

A field holds a value of the wrong type.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Convert the value before passing it: Univo never converts types implicitly, so 3 and "3" are different values.

Example:

```text
field `name` must be a string, found an integer
hint: Convert the value before passing it: Univo never converts types implicitly, so 3 and "3" are different values.
code: record.wrong_type (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordwrong_type)
```

### record.unsupported_type

A field holds a value of a type Univo cannot represent.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Convert the value to a string, integer, float, boolean, null, list or object.

Example:

```text
field `created_at` holds a value of unsupported type `datetime.datetime`
hint: Convert the value to a string, integer, float, boolean, null, list or object.
code: record.unsupported_type (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordunsupported_type)
```

### record.non_finite_number

A field holds NaN or an infinite number.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Replace NaN and infinities with a finite number or with null.

Example:

```text
field `score` holds NaN or an infinite number
hint: Replace NaN and infinities with a finite number or with null.
code: record.non_finite_number (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordnon_finite_number)
```

### record.integer_out_of_range

A field holds an integer outside the signed 64-bit range.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Pass integers beyond 64 bits as strings, and declare the field accordingly.

Example:

```text
field `population` holds an integer outside the signed 64-bit range
hint: Pass integers beyond 64 bits as strings, and declare the field accordingly.
code: record.integer_out_of_range (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordinteger_out_of_range)
```

### record.missing_identifier

The record has no identifier.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Give every record a string or integer identifier at the schema's id path.

Example:

```text
the identifier `id` is absent
hint: Give every record a string or integer identifier at the schema's id path.
code: record.missing_identifier (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordmissing_identifier)
```

### record.missing_partition_key

The record has no value for a partition key.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Give every record a value for each partition key: records are only compared within one partition.

Example:

```text
the partition key `tenant` is null
hint: Give every record a value for each partition key: records are only compared within one partition.
code: record.missing_partition_key (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordmissing_partition_key)
```

### record.missing_required_field

A required field is absent or null.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Provide the field, or declare it optional (required=False) if records may lack it.

Example:

```text
required field `description` is absent
hint: Provide the field, or declare it optional (required=False) if records may lack it.
code: record.missing_required_field (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordmissing_required_field)
```

### record.duplicate_key

An object holds the same key twice.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Remove the repeated key.

Example:

```text
the object at `params` holds the key "retries" twice
hint: Remove the repeated key.
code: record.duplicate_key (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordduplicate_key)
```

### record.key_collision

Two keys of an object differ only by their Unicode encoding.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Keep one of the two keys: after Unicode normalization they are the same key.

Example:

```text
the object at `params` holds two keys that both become "café" after Unicode normalization
hint: Keep one of the two keys: after Unicode normalization they are the same key.
code: record.key_collision (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordkey_collision)
```

### record.invalid_hostname

A hostname ends with more than one dot.

- Kind: `InvalidRecord` (Python: `univo.InvalidRecordError`)
- Fix: Remove the extra trailing dots: a hostname may end with at most one.

Example:

```text
hostname field `domain` ends with more than one dot
hint: Remove the extra trailing dots: a hostname may end with at most one.
code: record.invalid_hostname (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordinvalid_hostname)
```

### limit.nesting_depth

A selected value is nested too deeply.

- Kind: `ResourceLimit` (Python: `univo.ResourceLimitError`)
- Fix: Flatten the value, select a nested path instead of the whole object, or raise Limits.max_depth (at most 128).

Example:

```text
field `params` is nested more than 16 levels deep
hint: Flatten the value, select a nested path instead of the whole object, or raise Limits.max_depth (at most 128).
code: limit.nesting_depth (https://github.com/leopoldch/univo/blob/main/docs/errors.md#limitnesting_depth)
```

### limit.record_size

The selected values of a record are too large.

- Kind: `ResourceLimit` (Python: `univo.ResourceLimitError`)
- Fix: Select fewer or smaller fields, or raise Limits.max_selected_bytes.

Example:

```text
the selected values reach at least 128 bytes at `name`, above the limit of 64 bytes
hint: Select fewer or smaller fields, or raise Limits.max_selected_bytes.
code: limit.record_size (https://github.com/leopoldch/univo/blob/main/docs/errors.md#limitrecord_size)
```

### state.schema_mismatch

Records prepared with different schemas were compared.

- Kind: `SchemaMismatch` (Python: `univo.SchemaMismatchError`)
- Fix: Prepare both records with the schema used for the comparison.

Example:

```text
the records were prepared with a schema that normalizes differently from the one used to compare them
hint: Prepare both records with the schema used for the comparison.
code: state.schema_mismatch (https://github.com/leopoldch/univo/blob/main/docs/errors.md#stateschema_mismatch)
```

### record.duplicate_id

A record identifier is already present.

- Kind: `DuplicateId` (Python: `univo.DuplicateIdError`)
- Fix: Give each record a distinct identifier; replacing an existing record requires upsert, when available.

Example:

```text
record 1: the record identifier is already present in the index or batch
hint: Give each record a distinct identifier; replacing an existing record requires upsert, when available.
code: record.duplicate_id (https://github.com/leopoldch/univo/blob/main/docs/errors.md#recordduplicate_id)
```

### limit.index_capacity

The index record limit would be exceeded.

- Kind: `ResourceLimit` (Python: `univo.ResourceLimitError`)
- Fix: Use a smaller index or raise its explicit record limit before adding this batch.

Example:

```text
the batch would bring the index to 3 records, above its limit of 2
hint: Use a smaller index or raise its explicit record limit before adding this batch.
code: limit.index_capacity (https://github.com/leopoldch/univo/blob/main/docs/errors.md#limitindex_capacity)
```

### model.invalid_spec

The model string is not a valid model reference.

- Kind: `ModelUnavailable` (Python: `univo.ModelUnavailableError`)
- Fix: Pass "default", a Hugging Face repository such as "org/name" or "org/name@revision", or a local path starting with /, ./, ../, ~ or file://.

Example:

```text
model "potion" is not a valid model reference: expected "default", "org/name[@revision]" or a local path
hint: Pass "default", a Hugging Face repository such as "org/name" or "org/name@revision", or a local path starting with /, ./, ../, ~ or file://.
code: model.invalid_spec (https://github.com/leopoldch/univo/blob/main/docs/errors.md#modelinvalid_spec)
```

### model.unavailable

The model is not available locally and cannot be downloaded.

- Kind: `ModelUnavailable` (Python: `univo.ModelUnavailableError`)
- Fix: Download the model beforehand with `python -m univo.models download <model>`, point to a local copy, or allow network access.

Example:

```text
model minishlab/potion-base-8M is not available: offline mode is on and the model is not in the cache
hint: Download the model beforehand with `python -m univo.models download <model>`, point to a local copy, or allow network access.
code: model.unavailable (https://github.com/leopoldch/univo/blob/main/docs/errors.md#modelunavailable)
```

### model.unsupported_format

The model files are not in a format Univo can run.

- Kind: `ModelUnavailable` (Python: `univo.ModelUnavailableError`)
- Fix: Use a Model2Vec model (config.json, model.safetensors, tokenizer.json), or distill a sentence-transformers model into one with model2vec.

Example:

```text
model sentence-transformers/all-MiniLM-L6-v2 cannot be loaded: the repository has no model.safetensors
hint: Use a Model2Vec model (config.json, model.safetensors, tokenizer.json), or distill a sentence-transformers model into one with model2vec.
code: model.unsupported_format (https://github.com/leopoldch/univo/blob/main/docs/errors.md#modelunsupported_format)
```

### model.download_failed

A model file could not be downloaded.

- Kind: `ModelUnavailable` (Python: `univo.ModelUnavailableError`)
- Fix: Check network access, the hub URL and the repository name, then retry; set a mirror with hub_url or HF_ENDPOINT if the hub is not reachable.

Example:

```text
downloading model.safetensors of model minishlab/potion-base-8M failed: connection timed out
hint: Check network access, the hub URL and the repository name, then retry; set a mirror with hub_url or HF_ENDPOINT if the hub is not reachable.
code: model.download_failed (https://github.com/leopoldch/univo/blob/main/docs/errors.md#modeldownload_failed)
```

### model.cache_locked

Another process holds the lock on this model in the cache.

- Kind: `ModelUnavailable` (Python: `univo.ModelUnavailableError`)
- Fix: Wait for the other download to finish: the operating system releases the lock when that process exits, even if it is killed.

Example:

```text
model minishlab/potion-base-8M is locked by another process (/cache/models/minishlab/potion-base-8M/bf8b0566.lock)
hint: Wait for the other download to finish: the operating system releases the lock when that process exits, even if it is killed.
code: model.cache_locked (https://github.com/leopoldch/univo/blob/main/docs/errors.md#modelcache_locked)
```

### model.too_large

The model files exceed the allowed download size.

- Kind: `ResourceLimit` (Python: `univo.ResourceLimitError`)
- Fix: Choose a smaller model, or raise the model size limit if this size is expected.

Example:

```text
model minishlab/potion-multilingual-128M needs 3000000000 bytes, above the limit of 2147483648 bytes
hint: Choose a smaller model, or raise the model size limit if this size is expected.
code: model.too_large (https://github.com/leopoldch/univo/blob/main/docs/errors.md#modeltoo_large)
```

### model.integrity

A model file does not match its expected size or checksum.

- Kind: `ModelIntegrity` (Python: `univo.ModelIntegrityError`)
- Fix: Delete the cached copy and download it again; if it keeps failing, the source or a proxy alters the file.

Example:

```text
file model.safetensors of model minishlab/potion-base-8M fails verification: size is 1024 bytes, expected 30236760
hint: Delete the cached copy and download it again; if it keeps failing, the source or a proxy alters the file.
code: model.integrity (https://github.com/leopoldch/univo/blob/main/docs/errors.md#modelintegrity)
```

### model.corrupt

The model files cannot be read, or the model produced invalid vectors.

- Kind: `ModelIntegrity` (Python: `univo.ModelIntegrityError`)
- Fix: Download the model again; if a local copy is used, check that it is complete and unmodified.

Example:

```text
model /opt/models/m is corrupt: a vector is not finite
hint: Download the model again; if a local copy is used, check that it is complete and unmodified.
code: model.corrupt (https://github.com/leopoldch/univo/blob/main/docs/errors.md#modelcorrupt)
```

### state.model_mismatch

Embeddings of different models were combined.

- Kind: `IndexCompatibility` (Python: `univo.IndexCompatibilityError`)
- Fix: Encode every record of an index with the same model and revision; to change model, rebuild the index.

Example:

```text
embeddings of model minishlab/potion-base-2M@389b9f64be5aa4ae7a6bc6fe95ef20ce485ae5da cannot be combined with those of model minishlab/potion-base-8M@bf8b056651a2c21b8d2565580b8569da283cab23
hint: Encode every record of an index with the same model and revision; to change model, rebuild the index.
code: state.model_mismatch (https://github.com/leopoldch/univo/blob/main/docs/errors.md#statemodel_mismatch)
```

### state.index_out_of_sync

An index was used with records it does not cover.

- Kind: `IndexCompatibility` (Python: `univo.IndexCompatibilityError`)
- Fix: Search an index with the record store it was built from, and update it after appending records.

Example:

```text
the index covers 1000 records, but the store holds 1200
hint: Search an index with the record store it was built from, and update it after appending records.
code: state.index_out_of_sync (https://github.com/leopoldch/univo/blob/main/docs/errors.md#stateindex_out_of_sync)
```

### state.index_poisoned

An index is unusable after an internal failure.

- Kind: `IndexCompatibility` (Python: `univo.IndexCompatibilityError`)
- Fix: Load the index again from its last snapshot, or rebuild it, and report the failure: a panic interrupted a change to it.

Example:

```text
the index was left unusable by an internal failure during a change
hint: Load the index again from its last snapshot, or rebuild it, and report the failure: a panic interrupted a change to it.
code: state.index_poisoned (https://github.com/leopoldch/univo/blob/main/docs/errors.md#stateindex_poisoned)
```

### parameter.threshold

A similarity threshold is outside its valid range.

- Kind: `InvalidParameter` (Python: `univo.InvalidParameterError`)
- Fix: Pass a threshold greater than 0 and at most 1.

Example:

```text
threshold 1.1 is not in (0, 1]
hint: Pass a threshold greater than 0 and at most 1.
code: parameter.threshold (https://github.com/leopoldch/univo/blob/main/docs/errors.md#parameterthreshold)
```

<!-- catalog:end -->
