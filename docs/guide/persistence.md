# Snapshots

```python
from univo import Finder

finder = Finder(offline=True)
finder.add(["Northwind Traders", "NORTHWIND TRADERS", "Contoso"])
finder.save("./company-index")

restored = Finder.load("./company-index", offline=True)
assert restored.stats()["records"] == 3
```

A `Finder` lives in memory. `save` writes all its records to disk at once, and changes made afterwards are lost unless you save again. There is no write-ahead log.

## What is saved

A snapshot holds the IDs, partition keys, compared fields in normalized form, and vectors. Fields outside the schema and the original spelling are not kept; scores and explanations come out the same after loading.

A snapshot is protected against corruption by checksums, but it is neither encrypted nor authenticated. On Unix, its files are readable only by their owner. Keep it as you would keep the data.

## Loading

`Finder.load` checks the files and restores the index without encoding anything again. A semantic snapshot loads the model it was built with, at the same revision, even if the default model has changed since. With `offline=True`, that model must already be in the cache.

A snapshot from a release with a different format, normalizer, scoring or search version is refused with `IndexCompatibilityError`. Rebuild it from the original records ([stability](../reference/stability.md)).

## Limits

| Option | Default | |
| --- | --- | --- |
| `save(path, max_snapshot_bytes=...)` | 512 MiB | Size of the files written |
| `load(path, max_snapshot_bytes=...)` | 512 MiB | Size of the files read |
| `load(path, max_records=...)` | 100,000 | Records restored; raise it for larger indexes |

## Concurrent use and disk space

One process saves at a time; a second one gets `state.snapshot_locked`. Readers always see a complete snapshot, even while a save is running.

Each save adds a new generation and keeps the old ones, so that open readers stay valid. Nothing is deleted automatically, so old generations still hold records removed since. To reclaim space, remove old `g-*` directories when no process uses the snapshot, keeping `CURRENT` and the generation it names.

Snapshots are meant for local disks, not network shares. How files are published and laid out: [snapshot format](../design/snapshot-format.md).
