# 0016. Local concurrency and check_or_add

- Status: Accepted
- Date: 2026-10-12
- Roadmap: U-017

## Context

[specs/05-rust-architecture.md §16](../../specs/05-rust-architecture.md) asks for:
- reads on a stable generation and changes serialized per instance, with a simple, correct lock;
- encoding outside the lock when possible;
- a `check_or_add` that searches and decides to insert in one local atomic section, returning `inserted`, `matched` or `review`.

It also states the essential limit: local atomicity does not cover an external database or independent processes.

Until U-016, the Python `Finder` held a reentrant lock around every call, searches included. The native index took `&mut self`, so the lock was also what kept two threads off it. Searches from several threads ran one at a time.

## Decision

- **Lock in Rust.** The native index holds its records and generation in a `RwLock`. Searches, `find`, `save` and `stats` take the shared lock and run in parallel; `add`, `upsert`, `remove` and `check_or_add` take the exclusive lock.
- **No lock waits on Python.** Locks are only taken with the Python interpreter released, and nothing holding a lock touches Python objects. Results are built as plain Rust data under the lock and converted to Python after it is released.
- **Encoding outside the lock.** `SearchIndex::encoder()` returns a `QueryEncoder`, holding the schema, model and embedding cache as shared handles, under a brief shared lock. Searches, `add` and `check_or_add` encode with it while holding no lock, then take their lock. Before using the embeddings, the index checks three things, so a vector is never used for a record or a field it was not computed for:
  - they come from its schema, which fixes the field layout (`state.schema_mismatch`);
  - they cover exactly the records given, one for a search or `check_or_add` (`state.schema_mismatch`);
  - they come from its model (`state.model_mismatch`).
- **`check_or_add` in the engine.** `SearchIndex::check_or_add` runs in one exclusive borrow:
  1. it compacts if needed, so the decision and the insertion see one layout;
  2. it searches with the precomputed embeddings;
  3. it decides `Matched` when an active record is equal on every selected field after normalization (verified equality, as the exact lane decides it), `Review` when other records reach the minimum score, and otherwise `Inserted`, reusing the same embeddings for the insertion.
  
  On error nothing is inserted, and the generation advances only on insertion.
- **Bounded searches still insert.** When the search was bounded or approximate and found nothing, the record is inserted and the report says the search was bounded. Refusing would make `check_or_add` unusable with candidate retrieval, and the specification says the absence of candidates in a bounded mode is not a proof of absence.
- **A poisoned lock is an error.** If a panic ever interrupted a change, the lock is poisoned and every later call returns `state.index_poisoned`; the state is never used silently.
- **Python.** `Finder` keeps a small lock only for lazy initialization (model, cache, index). `Finder.check_or_add` returns a `Decision(status, id, report)`.

## Alternatives

- **Keep the Python lock around every call.** Simple and correct, but searches stay serialized, and the guarantee would live in the Python wrapper, where a Java binding could not reuse it. Rejected.
- **`check_or_add` as search then `add` in the wrapper.** Not atomic: in the race test below, eight threads inserted one to five copies of the same record per round. Rejected.
- **Encode under the exclusive lock.** Correct, but writers would hold the lock during model inference and block every reader. Rejected; `upsert` still encodes under the lock (see Consequences).
- **A lock-free or versioned (copy-on-write) index.** Readers would never wait for writers, at the cost of copying or versioning every lane. The specification prefers a simple correct lock first; revisit if write latency blocks readers in measured workloads.
- **Two outcomes only (`inserted` / `matched`).** Collapses verified equality and resemblance, which the vocabulary of [specs/02-python-api.md §7](../../specs/02-python-api.md) keeps apart (`exact` versus `candidate`). Rejected.

## Evidence

Apple M2, 8 cores, release build, Python 3.14. Lexical `Finder` of 100,000 synthetic company names, 2,000 searches at a minimum score of 0.8, best of three runs:

| Threads | Before (Python lock) | After (`RwLock`) |
| --- | --- | --- |
| 1 | 254 searches/s | 257 searches/s |
| 2 | 256 searches/s | 510 searches/s |
| 4 | 256 searches/s | 968 searches/s |

Races (8 threads released together by a barrier, each submitting the same name or a variant with a typo, over several rounds):
- **In Rust,** with the index behind a `RwLock`:
  - search then `add` (not atomic) inserted 7 or 8 records per round;
  - `check_or_add` inserted exactly 1 in every round.
- **In Python:**
  - search then `add` inserted 1 to 5 records per round;
  - `Finder.check_or_add` inserted exactly 1 in every round.

## Consequences

- One `Finder` can be shared by the threads of a server. Searches scale with threads, and writers wait for running searches before changing the index.
- `check_or_add` is atomic for one `Finder` in one process only. Several processes, each with its own `Finder`, or an application writing to a database, must enforce uniqueness themselves: a unique constraint, a transaction, or idempotent writes ([concurrency](../concurrency.md)).
- `upsert` still encodes under the exclusive lock, because it decides per field which embeddings it can reuse; searches wait during its encoding.
- The lock lives in the Python binding's native index. The engine provides the primitives (`QueryEncoder`, `search_encoded`, `add_encoded`, `check_or_add`), which a Java binding would wrap the same way.
- The new error code `state.index_poisoned` is part of the public catalog.

## Revisit when

- Write latency under the lock blocks readers in a measured workload: consider versioned (copy-on-write) publication.
- A second binding (Java) lands: move the lock and the generation from the Python binding into a shared engine type.
- `upsert` encoding becomes a measured bottleneck: precompute its embeddings outside the lock like `add`.
