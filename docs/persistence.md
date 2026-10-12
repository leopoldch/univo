# Explicit snapshots

```python
from univo import Finder

finder = Finder(offline=True)
finder.add(["Amazon", "AMAZON", "Microsoft"])
finder.save("./company-index")
restored = Finder.load("./company-index", offline=True)
assert restored.stats()["records"] == 3
```

`add` commits only to the in-memory Finder. `save` publishes a complete generation;
changes after it remain volatile until another successful save. There is no WAL.
`find` uses a temporary dataset and does not add it to the saved index. Saving or
loading is explicit I/O; a semantic snapshot may load its model even when empty.
Import and Finder construction remain free of model loading and network access.

Only IDs, normalized partition keys, selected normalized fields, normalization
explanation fingerprints and vectors are saved. Unselected input is discarded.
Original spelling/order is not retained; explanations and match decisions are.
Text and vectors remain sensitive. This format provides corruption detection,
not encryption or authentication of files supplied by an adversary.

## API and limits

- `finder.save(path, max_snapshot_bytes=512 << 20)` writes a new generation.
- `Finder.load(path, offline=False, cache_dir=None, hub_url=None, threads=1,
  max_snapshot_bytes=512 << 20, max_records=100_000)` restores it.
- The byte budget applies to the current generation's records, vectors, manifest
  and complete marker, not all historical generations or total process RSS.
- Load also caps records by the smaller of its explicit limit and the saved
  Finder capacity. Raise `max_records` explicitly to restore a larger index.
- Manifest size is at most 1 MiB; model dimensions must be in `[1, 65536]`.
  Conservative metadata preflights reject oversized schema/model strings before
  JSON copies and model I/O. Core schema depth/selected-value limits apply, with an encoded allowance for
  normalization expansion and wire metadata. Lengths and vector buffer products
  are checked against remaining file bytes before allocating; counts are not
  trusted as allocation hints for record or nested-value collections.

The destination's parent directory must exist. On Unix, newly created snapshot
roots/generations use mode 0700 and files use 0600, subject to umask. Existing root
permissions are not modified. Windows uses inherited filesystem ACLs; choose an
appropriately restricted parent directory.

## Publication and concurrency

Each save acquires `WRITER.lock` with a nonblocking OS file lock. Another writer
receives `IndexCompatibilityError` (`state.snapshot_locked`); locks are released
on return or process exit. Keep the lock file in place. Finder's own lock also
serializes its operations within that instance; this does not provide database
or distributed atomicity.

Save writes immutable `g-<time>-<pid>-<counter>` directories, flushes/syncs each
file, verifies SHA-256 checksums and completion, then atomically replaces `CURRENT`
with a synced temporary pointer in the same directory. A reader opens CURRENT
once and holds that generation's files. Concurrent later saves cannot mix its
records and vectors. Before publication, an error leaves the previous CURRENT;
after publication, an error can leave the complete new generation current.
Retry or load to discover which complete generation is visible.

Unix additionally syncs generation, root and parent directories. Windows syncs
files and replaces the pointer through Rust's standard filesystem API; the
standard library supplies no directory-sync operation there. No Windows promise
of directory-entry survival after power loss is made. Filesystem, OS and device
write-cache guarantees still govern physical durability. Process-exit injection
is tested; actual power interruption is not. This implementation targets local
filesystems; network shares and external modification of a live generation are
outside the supported concurrency contract.

Completed old generations are retained so open readers remain valid. Ordinary
failed saves clean up their unpublished generation; a terminated process can
leave an orphan. There is no automatic garbage collection in U-014. Repeated
saves therefore consume cumulative disk space; remove obsolete generations only
when no readers/writers use the snapshot directory, keeping CURRENT and its
referenced generation.

## Compatibility and offline loading

The manifest fixes format, normalizer, scoring/search versions, full schema,
model backend/dimension and model source/file identities. Hub models are resolved
at the saved commit, even if the original reference was `default` or unpinned.
Local models must remain at their resolved path with identical files. Models are
not bundled into snapshots. Offline load uses the verified local cache or local
model path and never attempts HTTP; a missing model raises `ModelUnavailableError`.

Checksums cover the bytes decoded, with headers, sizes, counts, field states,
canonical values, vector offsets, finiteness and unit norms checked before
exposure. Secondary exact/lexical/value/partition indexes are rebuilt; saved
vectors are installed without inference or a second whole-matrix copy. The
embedding memoization cache starts empty. IDs, insertion order, generation,
weights, vetoes, missingness and matching explanations are preserved.

Incompatible state is refused with an instruction to rebuild from original
records. There is no migration or silent model/normalizer substitution. Thresholds
are search-call options and may change without reencoding; a previously truncated
report cannot recover missing candidates by itself. Error codes are listed in the
[error catalogue](errors.md). See [ADR 0013](adr/0013-versioned-snapshots.md).

## Version-1 wire layout

All integers/floats in binary files are little-endian. Counts/string byte lengths
are u64. Strings are valid UTF-8; signed IDs and integer values are i64, exact
float values f64, semantic vectors f32. No platform-native struct layout, pickle
or third-party internal representation is persisted.

| File | Layout |
| --- | --- |
| `CURRENT` | Generation basename, newline, manifest SHA-256, newline; at most 256 bytes |
| `manifest.json` | Format/generation/normalizer/scoring/search versions, schema and preparation fingerprint, binding configuration, model identity, record count, dimension, sizes and SHA-256 of both data files; at most 1 MiB |
| `COMPLETE` | 64 lowercase ASCII hexadecimal bytes: SHA-256 of manifest |
| `records.bin` | `UNIVOR01`, u64 record count, records in slot order |
| `vectors.bin` | `UNIVOV01`, u64 schema field count; field presence u8; for each present field: u64 dimension/unique-row-count/record-count, u32 row references per record (`0xffffffff` absent), contiguous row-major f32 values |

Each record contains an ID tag (0 absent, 1 i64, 2 length+string), schema-ordered
partition values and compared fields. Fields use state tags 0 absent, 1 null,
2 length+normalized text, 3 canonical typed value; each includes a transform mask
u8 and three explanation-stage u64 fingerprints. Mask bits are Unicode form,
case, trailing dot and key order in that order. Typed values use tags 0 null,
1 bool u8 (0/1), 2 i64, 3 f64, 4 string, 5 list, 6 map. Lists contain count+values;
maps count+(length+key+value), with NFC keys sorted uniquely. Trigrams are rebuilt.

Schema metadata uses paths as arrays of literal keys, ordered fields with text
or exact options, optional ID path, partition paths and explicit limits. Binding
configuration preserves the public Python schema, original model reference,
pinned resolved reference, capacity and find/cache budgets. Fixed profile
versions identify the current generic scoring/search policy. These owned wire
representations belong to format 1; changing their meaning requires a new format
version. The preparation fingerprint is an additional compatibility check for
this version, never the sole proof of compatibility.
