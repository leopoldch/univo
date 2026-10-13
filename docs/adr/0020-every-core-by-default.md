# 0020. Every core by default, one pool per Finder

- Status: Accepted
- Date: 2026-10-13
- Roadmap: U-003 follow-up

## Context

Since [ADR 0018](0018-parallel-all-pairs.md), `find` and `group` run on `threads` workers, but `threads` defaulted to 1. SemHash uses every core by default, so a user comparing the two without tuning compared one Univo thread with every SemHash core. The maintainer decided that Univo uses every core by default.

Two costs stood in the way:
- Each native index created its own pool. `find` and `group` build a temporary index per call, so every call created and destroyed `threads` OS threads: 25 to 50 µs on a 20-record call at 4 threads.
- Lexical searches and `add` handed their (empty) encoding to the pool whenever one existed.

## Decision

- **Default.** `Finder(threads=None)` and `Finder.load(threads=None)` use every core the process may run on: Rust's `available_parallelism`, which follows CPU affinity and, on Linux, container CPU quotas. `config()["threads"]` reports the resolved number. An explicit positive integer keeps its meaning.
- **One pool per Finder.** `univo._univo.WorkerPool` is created once, on first need, and passed to every index of the Finder: the persistent one, those of `find` and `group`, and the one `load` restores. It is needed with semantic fields (encoding) or more than one thread.
- **Pool only when it helps.**
  - Encoding goes through the pool only when the schema has semantic fields.
  - `find` and `group` use the pool from 512 records; smaller reports run on the calling thread, where waking the workers would cost more than the work.
  - Single searches run on the calling thread, as before.

## Alternatives

- **Keep 1 by default:** predictable, but it leaves most of the machine idle and loses default comparisons. Rejected by the maintainer.
- **One process-wide pool:** fewer threads when a program builds many Finders, but Finders would share and wait for each other's workers, and two Finders could not have different budgets. Rejected.
- **`os.cpu_count()` in Python:** it ignores CPU affinity and container quotas, so a container limited to 2 CPUs on a 64-core host would start 64 threads. Rejected for `available_parallelism`.

## Evidence

20-record lexical calls, median of five runs of 2,000 calls, Apple M2 (8 cores):

| Threads | `find` | `group` |
| --- | --- | --- |
| 1 before (`main`) | 510 µs | 85 µs |
| 4 before, one pool per index | 531 µs | 134 µs |
| default (8), this change | 480 µs | 83 µs |
| 1, this change | 479 µs | 83 µs |

Tests (`tests/python/test_concurrency.py`):
- the default equals `available_threads()`, and `threads` values 0, -1, 1.5, `"2"` and `True` are refused;
- a Finder creates its pool once across `find`, `group`, `add` and `find`;
- a one-thread lexical Finder creates none;
- `find` and `group` give the same results on 1 and 4 threads, at 600 records, which is above the parallel threshold.

## Consequences

- A program that starts one Univo process per core, such as `multiprocessing` workers, now runs up to cores² threads unless it passes `threads=1` or its share. The [concurrency guide](../concurrency.md) says so.
- Concurrent `find` calls on one Finder share its workers instead of each having its own.
- Measurements in this repository state their thread count; earlier one-thread results stay valid with `threads=1`.

## Revisit when

- Users run many Finders in one process and the idle threads matter: share pools between Finders with equal budgets.
- Free-threaded Python or subinterpreters are supported: re-check thread counts and the 512-record threshold.
