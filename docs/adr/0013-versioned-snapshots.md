# 0013. Explicit versioned snapshots

- Status: Accepted (U-014)
- Date: 2026-10-12
- Roadmap: U-014; prerequisite U-011 merged

## Context

Finder stores prepared selected fields and compact f32 rows in Rust. Persistence
must preserve typed IDs, partitions, vetoes, missingness and explanations without
retaining unselected data or encoding the corpus again. U-014's acceptance is a
round trip with unchanged decisions, plus failure injection that exposes only
one complete generation. No WAL, mutations, ANN, migration or bundled-model
export belongs in this task.

## Decision

Use an explicit owned format 1: bounded JSON manifest, little-endian tagged
prepared-record stream and compact row-major f32 matrices/row references. Store
normalization-stage fingerprints; regenerate trigrams and secondary indexes.
A single nonblocking standard-library file lock protects each destination.
Write immutable generations, flush/sync files, verify SHA-256 and completion,
then replace CURRENT with a synced same-directory temporary pointer. Open readers
keep generation file handles. A failure after publication may expose the complete
new generation even when save returns an error. Complete old generations remain
until manually removed when no readers/writers are active.

Bind the full schema, normalizer/scoring/search versions, dimension, backend and
model source/file identities. Hub restoration resolves the saved commit rather
than a mutable alias. Local model path and files must match. Offline loading
uses verified local files only. The in-memory embedding cache is not persisted.
Restore compact matrices directly into the empty index, avoiding inference and a
second entire matrix allocation. The standard-workload spike also exposed corpus-
sized temporary encoding/cache rows during add. Bound these to at most 1,024
texts and about 1 MiB of float output per call (one row minimum), checking model
identity at each boundary. Deduplication and final field buffers remain global.
A real-model differential test confirms bit-identical rows against a whole
backend batch, with absent rows and duplicates covered separately. Late failure
still leaves the index unchanged; earlier cache publication may remain, as in
U-013. Constructing an empty index now reads only the
embedder descriptor, with zero-dimension validation, rather than encoding an
empty batch; the model-change oracle fixture is adjusted to count actual batches.

Default generation byte budget: 512 MiB; manifest limit: 1 MiB; default restored
record cap: 100,000, further bounded by saved capacity; dimension at most 65,536.
Read checks lengths, remaining bytes, checked buffer products, tags, canonical
values, unique typed IDs, row offsets and finite unit norms before exposure.
These bounds do not promise a total RSS limit: decoded fields, lanes, model and
caller inputs need additional memory. This is corruption detection, not an
untrusted-file authentication mechanism.

On Unix sync files and generation/root/parent directories. Windows gets file sync
and pointer replacement, without a directory-sync/power-loss directory-entry
promise. Physical durability remains dependent on OS/filesystem/device behavior.
Rust documents [replacement behavior](https://doc.rust-lang.org/std/fs/fn.rename.html)
and [file synchronization](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all).
This task tests process interruption rather than cutting power. Local filesystems
are in scope; network filesystems and external mutation of live generation files
are not supported concurrency modes.

## Alternatives

- Serialize all index internals: shorter initially, but dependent on collection
  layouts and crate versions; requires migration and preserves redundant postings.
- JSON matrices: simpler inspection, substantially larger vectors and slower
  parsing, with another whole-matrix object representation.
- Store selected original inputs in every PreparedRecord and reencode on load:
  adds steady-state copies and makes restoring large semantic corpora expensive.
- SQLite, a WAL or an external persistence library: adds dependencies and a
  transaction/migration surface beyond explicit snapshot requirements.
- Quantization/mmap: may reduce memory or loading time, but needs quality,
  alignment/lifetime and portable performance evidence before adoption.

## Evidence

The codec round-trips typed/nested data and explanation stages at the maximum
supported depth. Truncated prefixes and 1,024 bounded property-generated binary
inputs exercise rejection without panic. Native tests inject I/O errors and
abrupt child-process exits at twelve save boundaries, confirm stable opened
readers and lock release/refusal, and reject corrupt checksums, changed versions,
model identity, forged dimensions/offsets and NaN even with refreshed checksums.
Dimension-boundary tests round-trip an empty 65,536-dimensional index and reject
65,537 dimensions before directory creation or CURRENT replacement. A child
process using a simple relative path tests parent-directory sync failure after
complete publication, with current-directory changes isolated from other tests.
A counting embedder confirms zero inference calls during restore, including
shared rows and absent vectors.

Installed-wheel tests cover full reports, generations, appends after restore,
unselected-data omission, private Unix permissions, strict offline hub-commit
resolution and changed local files. The same release wheel is tested outside the
repository on CPython 3.12 and 3.14, without Rust in PATH. Native six-platform/MSRV
CI already runs these suites; cross-platform outcomes are not local evidence.

See [performance measurements](../performance.md) and the
[reproduction script](../../benchmarks/performance/python_snapshots.py) for the
format spike on 1k/10k/100k standard-workload synthetic records. Model footprint
and all end-to-end save/load samples, including model verification/loading and
lane reconstruction, are reported rather than an isolated file read.

At 100k, whole-process peak RSS over five cycles is 960.8 MiB after bounded
encoding, versus 1,087.0 MiB before; a single cycle peaks at 629.1 MiB. This scope
includes Python inputs/model/allocator lifetimes and does not isolate the 512 MiB
incremental engine target. That target remains open and unchanged, with further
profiling required before any budget-compliance release claim.

No new transitive/native package or Python runtime dependency: serde/serde_json
and SHA-256 reuse packages already linked by univo-embed. The macOS arm64 wheel is
approximately 1.742 MiB compressed, versus 1.673 MiB for reviewed U-013.

## Consequences

Snapshots are explicit durability boundaries with retained-generation disk
costs. Normalized selected data remains sensitive; original formatting cannot be
reconstructed from the snapshot. Model files are separate prerequisites. Format
or schema/model/normalizer incompatibility means a clear refusal and rebuilding
from source records. Query thresholds remain adjustable without reencoding.
There is no recovery guarantee for each volatile insertion or incomplete report.

## Revisit when

Measurements show matrix parsing or lane rebuilding dominates real workloads;
retained generations need automatic reclamation; durable Windows directory
publication is required; model relocation/export, migration, mmap, quantization
or stronger authenticated-file handling becomes an explicitly scoped task.
