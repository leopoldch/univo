# Snapshot format

How [snapshots](../guide/persistence.md) are written and read.

## Publication

A save writes a new directory, `g-<time>-<pid>-<counter>`, syncs each file, checks the SHA-256 checksums, then replaces `CURRENT` atomically with a synced temporary file. A reader opens `CURRENT` once and keeps the files of that generation, so a later save never mixes its records and vectors.

- **One writer.** Each save takes a non-blocking lock on `WRITER.lock`. A second writer gets `state.snapshot_locked`.
- **Failures.** An error before publication leaves the previous generation current. After publication, the new one may be current: load to find out.
- **Durability.** On Unix, the generation, root and parent directories are synced too. Windows has no directory sync in Rust's standard library, so a power loss there may lose the last publication. Process kills are tested; power loss is not.
- **Local filesystems only.** Network shares, and changes made to a live generation by other programs, are not supported.

## Loading

Every byte decoded is covered by a checksum. Headers, sizes, counts, field states, values, vector offsets, finiteness and unit norms are checked before use, and sizes are checked against the remaining bytes before any allocation. Exact, lexical and partition indexes are rebuilt; vectors are installed as saved, without inference. The embedding cache starts empty.

An approximate index is not saved: the manifest records its settings in `ann`, and the graph is rebuilt from the vectors. Its results may differ from before the save; exhaustive results do not.

## Layout, format 1

Binary values are little-endian. Counts and string lengths are u64, strings are UTF-8, IDs and integers i64, exact floats f64 and vectors f32. Nothing platform-specific, no pickle and no third-party internal layout is stored.

| File | Content |
| --- | --- |
| `CURRENT` | Generation name and manifest SHA-256, one per line; at most 256 bytes |
| `manifest.json` | Format, normalizer, scoring and search versions; schema and its fingerprint; Finder configuration; model identity; record count, dimension, and size and SHA-256 of both data files. At most 1 MiB |
| `COMPLETE` | SHA-256 of the manifest, in hexadecimal |
| `records.bin` | `UNIVOR01`, record count, records in slot order |
| `vectors.bin` | `UNIVOV01`, field count, then for each semantic field: dimension, unique rows, records, a u32 row per record (`0xffffffff` when absent), and the rows |

**Records.** An ID tag (0 none, 1 integer, 2 string), the partition values, then each compared field: a state (0 absent, 1 null, 2 normalized text, 3 typed value), a mask of the normalizations applied (Unicode form, case, trailing dot, key order) and three explanation fingerprints. Typed values are tagged 0 null, 1 bool, 2 i64, 3 f64, 4 string, 5 list, 6 map; map keys are NFC and sorted. Trigrams are not stored.

**Manifest.** Field paths are arrays of keys. The Finder configuration keeps the Python schema, the model reference as given and as resolved, the capacity and the budgets. Changing the meaning of any of these needs a new format version; the schema fingerprint is an extra check, not the only one.
