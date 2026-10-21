# Concurrency and check_or_add

## One Finder, many threads

A `Finder` can be shared by the threads of one process.

- **Searches run in parallel:** `search`, `find`, `save` and `stats` hold a shared lock and do not wait for each other.
- **Changes run one at a time:** `add`, `upsert`, `remove` and `check_or_add` hold the exclusive lock. They wait for running searches, and searches wait for them, so every call sees one complete generation.
- **Encoding happens outside the lock:** the semantic fields of searches, `add` and `check_or_add` are encoded before the lock is taken, so a slow model run does not block readers. `upsert` still encodes under the lock.
- **No lock is ever held while waiting for the Python interpreter:** long Rust work releases the interpreter, so other Python threads keep running.

On the reference machine, a lexical `Finder` of 100,000 names serves about 257 searches per second on one thread, 510 on two and 968 on four. Before parallel searches, a Python lock kept it at 256 whatever the number of threads.

`threads` is the size of the Finder's worker pool, which runs semantic encoding and the all-pairs reports `find` and `group`; their results do not depend on it. By default it is every core the process may use. The pool is created once per Finder and shared by its indexes, so concurrent `find` calls on one Finder share its workers. Its threads start only at the first semantic encoding or the first report large enough to run in parallel. A single search (`search`, `check_or_add`) runs on the calling thread, and so do reports over fewer than 512 records.

Processes that each run a Finder, such as `multiprocessing` workers, should pass `threads=1` (or their share of the cores) to avoid running more threads than cores.

## check_or_add

```python
from univo import Finder, Schema, Text

finder = Finder(schema=Schema(fields={"name": Text()}, id_field="id"))
decision = finder.check_or_add({"id": "org-1", "name": "Acme Corporation"})
assert (decision.status, decision.id) == ("inserted", "org-1")

decision = finder.check_or_add({"id": "org-2", "name": "ACME corporation"})
assert decision.status == "matched"  # equal after normalization; not inserted
decision = finder.check_or_add({"id": "org-3", "name": "Acme Corporatoin"})
assert decision.status == "review"  # similar enough to need a decision; not inserted
```

`check_or_add(record, *, min_score=0.8, limit=10, max_entries=None, exhaustive=False)` searches for the record and inserts it only if no stored record reaches `min_score`. The search and the insertion are one step under the exclusive lock, so two threads submitting the same duplicate never both insert it.

| `status` | Meaning | Inserted |
| --- | --- | --- |
| `inserted` | No stored record reached `min_score` | yes; `id` is the new record's ID, or its slot without an `id_field` |
| `matched` | A stored record is equal on every selected field after normalization | no |
| `review` | Records reach `min_score` without being equal; the application decides | no |

`decision.report` lists the matches the decision was based on, ranked as `search` ranks them:
- **Generation:** it advances only when the record is inserted.
- **Errors:** a duplicate ID, an invalid parameter or a model failure raises an error, and nothing is inserted.
- **Scores:** `min_score` is an uncalibrated ranking threshold, not an identity decision; `matched` is verified equality of the selected fields, not proof of the same real-world entity.

**A bounded search is not a proof.** With candidate retrieval (the default), and above all with `retrieval="hnsw"`, a duplicate can be missed when a lane reaches its work limit or the approximate search misses it. The record is then inserted, and the report says so: `search_profile` is `bounded` or `approximate`, and `candidate_truncated` and `warnings` report saturation. Use `exhaustive=True` to score every record of the partition.

## What it does not protect

`check_or_add` is atomic for **one `Finder` in one process**. It does not coordinate:
- several processes, or several `Finder` instances loaded from one snapshot, each with its own index;
- an external database: a record checked by Univo can be written to the database by another service in between.

A distributed application must enforce uniqueness where the data lives: a unique constraint, a transaction, or idempotent writes keyed by a stable ID. Univo then serves as the detector that proposes the matches, not as the arbiter.

Snapshots refuse concurrent writers with an operating-system lock ([persistence](persistence.md)); that lock protects the files, not the decisions of separate processes. Do not share a `Finder` across `fork()`: create it in the child process.
