# Groups

The basic result of Univo is a pair: two records and the evidence that they match. Groups are an optional view on top of pairs, and they are **never presented as entities**: similarity is not transitive, so A close to B and B close to C does not make A close to C.

```python
from univo import Finder, Schema, Text

finder = Finder(schema=Schema(fields={"name": Text()}, id_field="id"))
groups = finder.group(
    [
        {"id": 1, "name": "Acme Corporation"},
        {"id": 2, "name": "ACME corporation"},
        {"id": 3, "name": "Acme Corporatoin"},
        {"id": 4, "name": "Globex"},
    ]
)
assert groups.exact == ((1, 2),)  # equal after normalization
assert [group.members for group in groups.groups] == [(1, 2, 3)]
```

`Finder.group(records, *, min_score=0.8, max_group_size=100, max_pairs=1_000_000, max_entries=None, exhaustive=False)` groups a temporary dataset, like `find`, without storing it.

## Two kinds of groups

| Field | Contents | Why it is sound |
| --- | --- | --- |
| `exact` | Records equal on every selected field after normalization (Unicode form, case, hostname dot, key order) | Equality is transitive |
| `groups` | Conservative groups: every pair of members reaches `min_score` without a veto, at most `max_group_size` records | No pair inside a group is assumed: each one was scored |

An exact group is also a conservative group when its members, which all score alike, reach `min_score` together. Their score is computed, not assumed: two identical semantic vectors can have a cosine just below 1, so at `min_score=1` such a group stays an exact group only, as `find` would refuse the pair. Partitions and vetoes apply as everywhere: records of different partitions, or contradicting on a discriminant field, are never grouped.

Each `Group` holds its `members`, in input order, and `min_score`, the lowest score among its pairs.

## How conservative groups are built

1. **Exact groups are formed first.** Records with the same normalized fields score alike against any other record, so each exact group then counts as a single node. A thousand identical records cost one search, not half a million pairs. Records without exact evidence (blank or missing selected values) are never exact matches; they are left alone at once, without scanning the other blank records.
2. **Qualifying pairs are collected:** pairs between nodes that reach `min_score` without a veto, through the same search as `find`.
3. **Pairs are taken by descending score, then input position.** Two nodes or groups are joined only if *every* pair across them qualifies and the result stays within `max_group_size`.

Consequences, to know before relying on groups:
- **Chains stay open.** If A–B and B–C qualify but A–C does not, B joins whichever partner comes first in the order above, and the other stays apart.
- **Order matters.** Another order could group differently; the order used is deterministic, so the same input always gives the same groups.
- **Split pairs.** `split_pairs` counts qualifying pairs whose records ended in different groups or in none, and a warning says so. Use `find` to see those pairs.
- **Cost.** Checking a join costs one lookup per pair across the two sides, `O(g²)` for a group of `g` records; `max_group_size` bounds it.

## Limits and report

- `max_pairs` bounds the qualifying pairs held in memory: pairs are folded into the pair graph as each search finds them, without keeping explanations, so memory stays within the budget and the error comes as soon as it is exceeded. Above it, `ResourceLimitError` (`limit.group_pairs`) is raised. Raise `min_score`, partition the records, or raise the limit.
- The scan budgets of `find` apply before any scoring: `max_exhaustive_pairs` with `exhaustive=True`, and `max_semantic_comparisons` for semantic candidates with `retrieval="hnsw"`.
- `search_profile`, `candidate_truncated` and `warnings` say whether the underlying search was exhaustive, bounded or approximate. With a bounded or approximate search, a missing pair keeps records apart, so groups are only ever *smaller*, never wrongly joined.
- `qualifying_pairs`, `split_pairs`, `candidates` and `processed` describe the work done.

Groups never merge or change records. A future `plan_merge` (after 1.0) would choose representatives and keep field provenance; groups only propose what to review together.

## Cost

`group` runs the searches of `find`, one per record minus exact duplicates, then joins pairs. On the reference machine (Apple M2, one thread), synthetic company names in one partition, one record in ten a near-duplicate, `min_score=0.8`:

| Records | `group` | `find` | Groups | Qualifying pairs | Split pairs |
| --- | --- | --- | --- | --- | --- |
| 10,000 | 2.6 s | 2.6 s | 1,007 | 1,012 | 5 |
| 20,000 | 10.3 s | 11.0 s | 2,041 | 2,062 | 13 |

Both grow faster than linearly in one partition, because every record is searched against the others; partitions and higher thresholds reduce the work. Since [ADR 0018](../adr/0018-parallel-all-pairs.md), each pair is scanned once and the searches run on the `threads` budget, with the same groups: see [performance](../development/performance.md#parallel-all-pairs-reports-adr-0018).
